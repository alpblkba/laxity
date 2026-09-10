mod experiment;
mod telemetry;

use std::{
    collections::VecDeque,
    env,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    net::{SocketAddr, UdpSocket},
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table, Wrap},
    DefaultTerminal, Frame,
};
use serialport::{
    DataBits, FlowControl, Parity, SerialPort, SerialPortInfo, SerialPortType, StopBits,
};
use tachyonfx::{fx, EffectManager, Interpolation};

use experiment::{
    Activity, ExperimentState, MemoryRegionState, Mode, Penalty, PlacementStats, Relation,
};
use telemetry::{ParseDelta, Parser, Placement, Stats, CELL_SAMPLE_LIMIT};

const FG: Color = Color::Rgb(220, 223, 228);
const DIM: Color = Color::Rgb(138, 145, 158);
const FAINT: Color = Color::Rgb(88, 94, 106);
const ACCENT: Color = Color::Rgb(233, 105, 168);
const ACCENT_ALT: Color = Color::Rgb(97, 214, 214);
const SUCCESS: Color = Color::Rgb(126, 209, 131);
const WARNING: Color = Color::Rgb(232, 183, 96);
const DANGER: Color = Color::Rgb(233, 106, 106);
const BORDER: Color = Color::Rgb(72, 78, 92);

const READ_SIZE: usize = 65_536;
const MAX_READS_PER_TICK: usize = 64;
const MAX_LOG_LINES: usize = 256;
const REPLAY_BYTES_PER_TICK: usize = 320;
const DISPLAY_HOLD: Duration = Duration::from_millis(450);
const RELATION_EFFECT_COOLDOWN: Duration = Duration::from_millis(1200);
const WIDE_MIN_WIDTH: u16 = 110;
const WIDE_MIN_HEIGHT: u16 = 34;
const RECORD_SEPARATOR: &str = "   ·   record ";

#[derive(Clone, Debug)]
enum SourceSpec {
    Udp(u16),
    Serial(Option<PathBuf>),
    File(PathBuf),
}

#[derive(Debug)]
struct Options {
    source: SourceSpec,
    record: Option<PathBuf>,
    headless: bool,
    duration: Option<Duration>,
}

enum Args {
    Run(Options),
    Help,
}

enum Input {
    Udp {
        socket: UdpSocket,
        port: u16,
        peer: Option<SocketAddr>,
    },
    Serial {
        port: Box<dyn SerialPort>,
        path: PathBuf,
    },
    File {
        file: File,
        path: PathBuf,
        done: bool,
    },
    Broken {
        label: String,
        error: String,
    },
}

enum ReadResult {
    Bytes(usize),
    Empty,
    Eof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Health {
    Waiting,
    Live,
    ReplayRunning,
    TextOnly,
    Rejecting,
    Stale,
    ReplayDone,
    ReplayInvalid,
    RecordError,
    SourceError,
}

struct RateMeter {
    window_started: Instant,
    window_bytes: u64,
    bytes_per_second: f64,
}

struct LogBuffer {
    lines: VecDeque<String>,
    partial: String,
}

struct App {
    source: Input,
    parser: Parser,
    logs: LogBuffer,
    recorder: Option<File>,
    record_path: Option<PathBuf>,
    record_label: String,
    record_error: Option<String>,
    last_byte: Option<Instant>,
    last_frame: Option<Instant>,
    last_reject: Option<Instant>,
    total_bytes: u64,
    rate: RateMeter,
    source_finished: bool,
    should_quit: bool,
    effects: EffectManager<&'static str>,
    header_area: Rect,
    memory_area: Rect,
    last_health: Option<Health>,
    display_state: Option<ExperimentState>,
    last_display_update: Option<Instant>,
    last_relation: Option<Relation>,
    last_relation_effect: Option<Instant>,
}

impl Input {
    fn open(spec: &SourceSpec) -> Self {
        match spec {
            SourceSpec::Udp(port) => match UdpSocket::bind(("0.0.0.0", *port)) {
                Ok(socket) => match socket.set_nonblocking(true) {
                    Ok(()) => Self::Udp {
                        socket,
                        port: *port,
                        peer: None,
                    },
                    Err(error) => Self::broken(format!("udp 0.0.0.0:{port}"), error),
                },
                Err(error) => Self::broken(format!("udp 0.0.0.0:{port}"), error),
            },
            SourceSpec::Serial(path) => {
                let path = match path.clone().map(Ok).unwrap_or_else(find_serial_port) {
                    Ok(path) => path,
                    Err(error) => {
                        return Self::Broken {
                            label: "serial auto · 921600 8N1".to_string(),
                            error,
                        }
                    }
                };
                let label = format!("serial {} · 921600 8N1", path.display());
                match serialport::new(path.to_string_lossy(), 921_600)
                    .data_bits(DataBits::Eight)
                    .parity(Parity::None)
                    .stop_bits(StopBits::One)
                    .flow_control(FlowControl::None)
                    .timeout(Duration::from_millis(2))
                    .open()
                {
                    Ok(port) => Self::Serial { port, path },
                    Err(error) => Self::Broken {
                        label,
                        error: error.to_string(),
                    },
                }
            }
            SourceSpec::File(path) => match File::open(path) {
                Ok(file) => Self::File {
                    file,
                    path: path.clone(),
                    done: false,
                },
                Err(error) => Self::broken(format!("file {}", path.display()), error),
            },
        }
    }

    fn broken(label: String, error: io::Error) -> Self {
        Self::Broken {
            label,
            error: error.to_string(),
        }
    }

    fn read(&mut self, buffer: &mut [u8]) -> io::Result<ReadResult> {
        match self {
            Self::Udp { socket, peer, .. } => match socket.recv_from(buffer) {
                Ok((0, _)) => Ok(ReadResult::Empty),
                Ok((count, address)) => {
                    *peer = Some(address);
                    Ok(ReadResult::Bytes(count))
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(ReadResult::Empty),
                Err(error) => Err(error),
            },
            Self::Serial { port, .. } => match port.read(buffer) {
                Ok(0) => Ok(ReadResult::Empty),
                Ok(count) => Ok(ReadResult::Bytes(count)),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) =>
                {
                    Ok(ReadResult::Empty)
                }
                Err(error) => Err(error),
            },
            Self::File { file, done, .. } => {
                if *done {
                    return Ok(ReadResult::Empty);
                }
                match file.read(buffer)? {
                    0 => {
                        *done = true;
                        Ok(ReadResult::Eof)
                    }
                    count => Ok(ReadResult::Bytes(count)),
                }
            }
            Self::Broken { .. } => Ok(ReadResult::Empty),
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Udp { port, peer, .. } => match peer {
                Some(peer) => format!("udp 0.0.0.0:{port} · peer {peer}"),
                None => format!("udp 0.0.0.0:{port} · peer waiting"),
            },
            Self::Serial { path, .. } => {
                format!("serial {} · 921600 8N1", path.display())
            }
            Self::File { path, .. } => format!("file {}", path.display()),
            Self::Broken { label, .. } => label.clone(),
        }
    }

    fn error(&self) -> Option<&str> {
        match self {
            Self::Broken { error, .. } => Some(error),
            _ => None,
        }
    }

    fn is_file(&self) -> bool {
        matches!(self, Self::File { .. })
    }

    fn fail(&mut self, error: io::Error) {
        let label = self.label();
        *self = Self::Broken {
            label,
            error: error.to_string(),
        };
    }
}

impl RateMeter {
    fn new(now: Instant) -> Self {
        Self {
            window_started: now,
            window_bytes: 0,
            bytes_per_second: 0.0,
        }
    }

    fn add(&mut self, bytes: usize) {
        self.window_bytes += bytes as u64;
    }

    fn update(&mut self, now: Instant) {
        let elapsed = now.duration_since(self.window_started);
        if elapsed >= Duration::from_millis(500) {
            self.bytes_per_second = self.window_bytes as f64 / elapsed.as_secs_f64();
            self.window_started = now;
            self.window_bytes = 0;
        }
    }
}

impl LogBuffer {
    fn new() -> Self {
        let mut out = Self {
            lines: VecDeque::new(),
            partial: String::new(),
        };
        out.push_line("waiting for source bytes".to_string());
        out
    }

    fn push_bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            match *byte {
                b'\n' => self.commit_partial(),
                b'\r' => {}
                b'\t' => self.partial.push(' '),
                0x20..=0x7e => self.partial.push(*byte as char),
                _ => self.partial.push('·'),
            }

            if self.partial.chars().count() >= 240 {
                self.commit_partial();
            }
        }
    }

    fn push_line(&mut self, line: String) {
        self.lines.push_back(line);
        while self.lines.len() > MAX_LOG_LINES {
            self.lines.pop_front();
        }
    }

    fn commit_partial(&mut self) {
        if !self.partial.is_empty() {
            let line = std::mem::take(&mut self.partial);
            self.push_line(line);
        }
    }

    fn visible(&self, height: usize) -> Vec<String> {
        let mut all: Vec<_> = self.lines.iter().cloned().collect();
        if !self.partial.is_empty() {
            all.push(self.partial.clone());
        }
        let start = all.len().saturating_sub(height);
        all.drain(..start);
        all
    }
}

impl App {
    fn new(options: &Options) -> Self {
        let now = Instant::now();
        let source = Input::open(&options.source);
        let mut logs = LogBuffer::new();
        if let Some(error) = source.error() {
            logs.push_line(format!("source open failed: {error}"));
        }

        let record_label = match options.record.as_deref() {
            Some(path) => path.display().to_string(),
            None => "replay only".to_string(),
        };

        Self {
            source,
            parser: Parser::default(),
            logs,
            recorder: None,
            record_path: options.record.clone(),
            record_label,
            record_error: None,
            last_byte: None,
            last_frame: None,
            last_reject: None,
            total_bytes: 0,
            rate: RateMeter::new(now),
            source_finished: false,
            should_quit: false,
            effects: EffectManager::default(),
            header_area: Rect::default(),
            memory_area: Rect::default(),
            last_health: None,
            display_state: None,
            last_display_update: None,
            last_relation: None,
            last_relation_effect: None,
        }
    }

    fn poll_source(&mut self) {
        if self.source_finished || self.source.error().is_some() {
            return;
        }

        let pace_replay = self.source.is_file();
        let mut buffer = [0u8; READ_SIZE];
        for _ in 0..MAX_READS_PER_TICK {
            let read_size = if pace_replay {
                REPLAY_BYTES_PER_TICK
            } else {
                READ_SIZE
            };
            match self.source.read(&mut buffer[..read_size]) {
                Ok(ReadResult::Bytes(count)) => {
                    self.ingest(&buffer[..count]);
                    if pace_replay {
                        break;
                    }
                }
                Ok(ReadResult::Empty) => break,
                Ok(ReadResult::Eof) => {
                    let delta = self.parser.finish();
                    let now = Instant::now();
                    self.accept_delta(delta, now);
                    self.update_display_record(now, true);
                    self.source_finished = true;
                    self.logs.push_line("replay complete".to_string());
                    break;
                }
                Err(error) => {
                    let message = error.to_string();
                    self.logs
                        .push_line(format!("source read failed: {message}"));
                    self.source.fail(error);
                    break;
                }
            }
        }
    }

    fn ingest(&mut self, bytes: &[u8]) {
        let now = Instant::now();
        self.open_recorder();
        if let Some(recorder) = self.recorder.as_mut() {
            if let Err(error) = recorder.write_all(bytes) {
                let message = error.to_string();
                self.record_error = Some(message.clone());
                self.recorder = None;
                self.logs
                    .push_line(format!("recorder write failed: {message}"));
            }
        }

        self.total_bytes += bytes.len() as u64;
        self.rate.add(bytes.len());
        self.last_byte = Some(now);
        let delta = self.parser.feed(bytes);
        self.accept_delta(delta, now);
    }

    fn update_display_record(&mut self, now: Instant, force: bool) {
        let Some(latest) = self.parser.latest_record else {
            return;
        };
        if self.display_state.as_ref().map(|state| state.record.seq) == Some(latest.seq) {
            return;
        }
        let ready = self
            .last_display_update
            .is_none_or(|last| now.duration_since(last) >= DISPLAY_HOLD);
        if force || ready {
            self.display_state = Some(ExperimentState::from_parser(&self.parser, latest));
            self.last_display_update = Some(now);
        }
    }

    fn open_recorder(&mut self) {
        if self.recorder.is_some() || self.record_error.is_some() {
            return;
        }
        let Some(path) = self.record_path.as_deref() else {
            return;
        };

        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => self.recorder = Some(file),
            Err(error) => {
                let message = error.to_string();
                self.record_error = Some(message.clone());
                self.logs
                    .push_line(format!("recorder open failed: {message}"));
            }
        }
    }

    fn finish_recording(&mut self) -> io::Result<()> {
        if let Some(recorder) = self.recorder.as_mut() {
            recorder.flush()?;
        }
        match self.record_error.as_deref() {
            Some(error) => Err(io::Error::other(format!("recording failed: {error}"))),
            None => Ok(()),
        }
    }

    fn accept_delta(&mut self, delta: ParseDelta, now: Instant) {
        if delta.valid_frames > 0 {
            self.last_frame = Some(now);
        }
        if delta.rejected_candidates > 0 {
            self.last_reject = Some(now);
        }
        self.logs.push_bytes(&delta.ascii);
    }

    fn health(&self, now: Instant) -> Health {
        if self.source.error().is_some() {
            return Health::SourceError;
        }
        if self.record_error.is_some() {
            return Health::RecordError;
        }
        if self.source_finished {
            return if self.parser.stats.accepted_frames > 0 {
                Health::ReplayDone
            } else {
                Health::ReplayInvalid
            };
        }
        let Some(last_byte) = self.last_byte else {
            return Health::Waiting;
        };
        if now.duration_since(last_byte) > Duration::from_secs(2) {
            return Health::Stale;
        }
        if self
            .last_frame
            .is_some_and(|last| now.duration_since(last) <= Duration::from_secs(2))
        {
            return if self.source.is_file() {
                Health::ReplayRunning
            } else {
                Health::Live
            };
        }
        if self
            .last_reject
            .is_some_and(|last| now.duration_since(last) <= Duration::from_secs(2))
        {
            Health::Rejecting
        } else {
            Health::TextOnly
        }
    }

    fn prepare_health_effect(&mut self, now: Instant) {
        let health = self.health(now);
        if self.last_health == Some(health) || self.header_area.is_empty() {
            return;
        }
        if self.last_health.is_some()
            || matches!(
                health,
                Health::Live
                    | Health::ReplayRunning
                    | Health::Rejecting
                    | Health::ReplayInvalid
                    | Health::RecordError
                    | Health::SourceError
            )
        {
            self.effects.add_unique_effect(
                "health",
                fx::fade_from_fg(health.color(), (180, Interpolation::SineOut))
                    .with_area(self.header_area),
            );
        }
        self.last_health = Some(health);
    }

    fn prepare_relation_effect(&mut self, now: Instant) {
        let Some(state) = self.display_state.as_ref() else {
            return;
        };
        let relation = state.relation;
        if self.last_relation == Some(relation) || self.memory_area.is_empty() {
            return;
        }

        let had_relation = self.last_relation.is_some();
        self.last_relation = Some(relation);
        let cooled_down = self
            .last_relation_effect
            .is_none_or(|last| now.duration_since(last) >= RELATION_EFFECT_COOLDOWN);
        if had_relation && cooled_down {
            self.effects.add_unique_effect(
                "relation",
                fx::fade_from_fg(relation_color(relation), (180, Interpolation::SineOut))
                    .with_area(self.memory_area),
            );
            self.last_relation_effect = Some(now);
        }
    }

    fn draw(&mut self, frame: &mut Frame, elapsed: Duration, now: Instant) {
        let screen = frame.area();
        let experiment = self.display_state.as_ref();

        if screen.width >= WIDE_MIN_WIDTH && screen.height >= WIDE_MIN_HEIGHT {
            let log_height = if screen.height >= 36 { 7 } else { 5 };
            let areas = Layout::vertical([
                Constraint::Length(7),
                Constraint::Min(12),
                Constraint::Length(log_height),
            ])
            .split(screen);
            let dashboard =
                Layout::vertical([Constraint::Min(9), Constraint::Length(9)]).split(areas[1]);
            let upper =
                Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)])
                    .split(dashboard[0]);

            self.header_area = areas[0];
            self.memory_area = upper[0];
            self.draw_status(frame, areas[0], now);
            self.draw_memory_topology(frame, upper[0], experiment);
            self.draw_current_job(frame, upper[1], experiment);
            self.draw_comparison(frame, dashboard[1], experiment);
            self.draw_log(frame, areas[2]);
        } else {
            let areas = Layout::vertical([
                Constraint::Length(7),
                Constraint::Min(7),
                Constraint::Length(5),
            ])
            .split(screen);
            self.header_area = areas[0];
            self.memory_area = areas[1];
            self.draw_status(frame, areas[0], now);
            self.draw_compact_dashboard(frame, areas[1], experiment);
            self.draw_log(frame, areas[2]);
        }

        self.effects
            .process_effects(elapsed.into(), frame.buffer_mut(), screen);
    }

    fn draw_status(&self, frame: &mut Frame, area: Rect, now: Instant) {
        let health = self.health(now);
        let record_style = if self.record_error.is_some() {
            DANGER
        } else {
            DIM
        };
        let record_label = if self.record_error.is_some() {
            format!("failed · {}", self.record_label)
        } else {
            self.record_label.clone()
        };
        let (source_label, record_label) = fit_header_pair(
            &self.source.label(),
            &record_label,
            area.width.saturating_sub(2) as usize,
        );
        let clock = self
            .parser
            .metadata
            .as_ref()
            .map(|metadata| format!("{:.3} MHz CYCCNT", metadata.cyccnt_hz as f64 / 1_000_000.0))
            .unwrap_or_else(|| "clock waiting".to_string());
        let lines = vec![
            Line::from(vec![
                Span::styled("● ", Style::default().fg(health.color())),
                Span::styled(
                    health.label(),
                    Style::default()
                        .fg(health.color())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("   ·   q quit", Style::default().fg(FAINT)),
            ])
            .alignment(Alignment::Center),
            Line::from(vec![
                Span::styled(source_label, Style::default().fg(ACCENT_ALT)),
                Span::styled(RECORD_SEPARATOR, Style::default().fg(FAINT)),
                Span::styled(record_label, Style::default().fg(record_style)),
            ])
            .alignment(Alignment::Center),
            Line::from(vec![
                metric(clock),
                separator(),
                metric(format_rate(self.rate.bytes_per_second)),
                separator(),
                metric(format!(
                    "{} frames",
                    comma(self.parser.stats.accepted_frames)
                )),
                separator(),
                metric(format!("{} records", comma(self.parser.stats.records))),
            ])
            .alignment(Alignment::Center),
            Line::from(vec![
                warning_metric(format!(
                    "{} crc rejects",
                    comma(self.parser.stats.crc_rejections)
                )),
                separator(),
                warning_metric(format!("{} seq gaps", comma(self.parser.stats.gaps))),
                separator(),
                warning_metric(format!(
                    "{} target drops",
                    comma(self.parser.stats.dropped as u64)
                )),
            ])
            .alignment(Alignment::Center),
            Line::from(vec![
                Span::styled("last byte ", Style::default().fg(FAINT)),
                Span::styled(age(self.last_byte, now), Style::default().fg(FG)),
                separator(),
                Span::styled("last valid frame ", Style::default().fg(FAINT)),
                Span::styled(age(self.last_frame, now), Style::default().fg(FG)),
                separator(),
                Span::styled(
                    format!("{} received", format_bytes(self.total_bytes)),
                    Style::default().fg(DIM),
                ),
            ])
            .alignment(Alignment::Center),
        ];

        frame.render_widget(
            Paragraph::new(lines).block(panel(" laxity ╱ memory telemetry ", health.color())),
            area,
        );
    }

    fn draw_memory_topology(&self, frame: &mut Frame, area: Rect, state: Option<&ExperimentState>) {
        let title = " logical SRAM topology ╱ traffic flow ";
        let Some(state) = state else {
            frame.render_widget(
                waiting_panel(
                    title,
                    "waiting for placement metadata",
                    "banks appear after a valid header",
                ),
                area,
            );
            return;
        };
        if state.regions.is_empty() {
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "record accepted · placement metadata unavailable",
                    Style::default().fg(WARNING).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!(
                        "inference label id{} · topology not inferred",
                        state.record.region_id
                    ),
                    Style::default().fg(DIM),
                )),
            ];
            frame.render_widget(
                Paragraph::new(lines)
                    .alignment(Alignment::Center)
                    .block(panel(title, WARNING)),
                area,
            );
            return;
        }

        let inference = placement_name(state.inference.as_ref(), state.record.region_id);
        let dma = state
            .aggressor
            .region_id
            .map(|id| placement_name(state.dma_target.as_ref(), id))
            .unwrap_or_else(|| "off".to_string());
        let mut lines = vec![
            flow_line("CPU / inference", &inference, ACCENT),
            flow_line("GPDMA traffic", &dma, WARNING),
            Line::from(Span::styled(
                relation_message(state.relation),
                Style::default()
                    .fg(relation_color(state.relation))
                    .add_modifier(Modifier::BOLD),
            ))
            .alignment(Alignment::Center),
        ];

        let detailed =
            area.height.saturating_sub(2) >= (state.regions.len() as u16).saturating_mul(2) + 4;
        for region in &state.regions {
            let color = activity_color(region.activity);
            let arena = region.active_arena.as_ref().unwrap_or(&region.placement);
            let activity = region_activity_label(region);
            let marker = if region.activity == Activity::Inactive {
                "·"
            } else {
                "▎"
            };
            if detailed {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{marker} {:<8}", region.placement.name),
                        Style::default().fg(color),
                    ),
                    Span::styled(
                        format!("  [{activity}]"),
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ),
                ]));
                lines.push(Line::from(Span::styled(
                    format!(
                        "    arena {} · {} · cell n {}",
                        arena_range(arena),
                        format_arena_size(arena.arena_size),
                        comma(region.samples as u64)
                    ),
                    Style::default().fg(DIM),
                )));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{marker} {:<7}", region.placement.name),
                        Style::default().fg(color),
                    ),
                    Span::styled(
                        format!(
                            " {} + {} ",
                            format_address(arena.arena_addr),
                            format_arena_size(arena.arena_size)
                        ),
                        Style::default().fg(DIM),
                    ),
                    Span::styled(format!("[{activity}]"), Style::default().fg(color)),
                ]));
            }
        }
        lines.push(
            Line::from(Span::styled(
                "logical regions from telemetry metadata · not die geometry",
                Style::default().fg(FAINT),
            ))
            .alignment(Alignment::Center),
        );

        frame.render_widget(Paragraph::new(lines).block(panel(title, BORDER)), area);
    }

    fn draw_current_job(&self, frame: &mut Frame, area: Rect, state: Option<&ExperimentState>) {
        let title = " current observed cell ╱ wall cycles ";
        let Some(state) = state else {
            frame.render_widget(
                waiting_panel(
                    title,
                    "waiting for an observed record",
                    "latency and arena appear after it",
                ),
                area,
            );
            return;
        };

        let clock_hz = observed_clock(&self.parser);
        let inference = placement_name(state.inference.as_ref(), state.record.region_id);
        let arena = state
            .inference
            .as_ref()
            .map(|placement| {
                format!(
                    "{} + {}",
                    format_address(placement.arena_addr),
                    format_arena_size(placement.arena_size)
                )
            })
            .unwrap_or_else(|| "metadata unavailable".to_string());
        let dma = match state.aggressor.region_id {
            None => "off".to_string(),
            Some(id) => format!(
                "{} · {}",
                placement_name(state.dma_target.as_ref(), id),
                footprint_label(state)
            ),
        };
        let mut lines = vec![
            key_value(
                "mode",
                format!(
                    "{} · {}",
                    mode_label(state.mode),
                    relation_label(state.relation)
                ),
                relation_color(state.relation),
            ),
            key_value("inference", inference, ACCENT),
            key_value("arena", arena, FG),
            key_value("DMA target", dma, WARNING),
            key_value(
                "latest",
                format_cycle_time(state.record.exec_cyc, clock_hz),
                FG,
            ),
            key_value("off p50", optional_cycles(state.baseline), ACCENT_ALT),
            key_value(
                "cell p50",
                state
                    .current_median
                    .map(|cycles| {
                        format!(
                            "{} · n {}",
                            comma_u32(cycles),
                            comma(state.current_samples as u64)
                        )
                    })
                    .unwrap_or_else(|| "waiting".to_string()),
                FG,
            ),
            key_value(
                "p50 penalty",
                state
                    .penalty
                    .map(format_penalty)
                    .unwrap_or_else(|| "waiting for own off cell".to_string()),
                penalty_color(state.penalty),
            ),
        ];
        if state.mode == Mode::Contention {
            let proof = match state.transfer_advanced {
                Some(true) => format!("advanced · counter {}", comma(state.record.reserved as u64)),
                Some(false) => format!("pending · counter {}", comma(state.record.reserved as u64)),
                None => "unavailable".to_string(),
            };
            lines.push(key_value("DMA proof", proof, DIM));
        }
        lines.push(Line::from(Span::styled(
            "wall cycles mix preemption + contention",
            Style::default().fg(FAINT),
        )));
        lines.push(Line::from(Span::styled(
            format!(
                "p50 windows retain ≤ {} samples/cell",
                comma(CELL_SAMPLE_LIMIT as u64)
            ),
            Style::default().fg(FAINT),
        )));

        frame.render_widget(Paragraph::new(lines).block(panel(title, BORDER)), area);
    }

    fn draw_comparison(&self, frame: &mut Frame, area: Rect, state: Option<&ExperimentState>) {
        let Some(state) = state else {
            frame.render_widget(
                waiting_panel(
                    " placement comparison ╱ exact experiment cells ",
                    "waiting for comparable experiment cells",
                    "p50 appears as each cell arrives",
                ),
                area,
            );
            return;
        };

        let minimum = state
            .comparisons
            .iter()
            .filter_map(|stats| stats.median)
            .min();
        let maximum = state
            .comparisons
            .iter()
            .filter_map(|stats| stats.median)
            .max();
        let max_penalty = state
            .comparisons
            .iter()
            .filter_map(|stats| stats.penalty)
            .map(|penalty| penalty.cycles.unsigned_abs())
            .max()
            .unwrap_or(1)
            .max(1);
        let max_median = maximum.unwrap_or(1).max(1);
        let bar_width = area.width.saturating_sub(99).clamp(8, 28) as usize;

        let rows = state.comparisons.iter().map(|stats| {
            let current = state.inference_bank_id == Some(stats.placement.id);
            let marker = if current { "▎" } else { " " };
            let best = if stats.observed_best {
                " · observed best"
            } else {
                ""
            };
            let change = comparison_change(state.mode, stats, minimum);
            let fill = match (state.mode, stats.median, stats.penalty) {
                (Mode::Baseline, Some(median), _) => {
                    (median as u64 * bar_width as u64 / max_median as u64) as usize
                }
                (Mode::Contention, _, Some(penalty)) => {
                    (penalty.cycles.unsigned_abs() * bar_width as u64 / max_penalty) as usize
                }
                _ => 0,
            }
            .min(bar_width);
            let row_color = if current {
                ACCENT
            } else if stats.observed_best {
                SUCCESS
            } else {
                region_color(stats.placement.id)
            };
            let bar_color = if state.mode == Mode::Baseline {
                row_color
            } else {
                penalty_color(stats.penalty)
            };

            Row::new(vec![
                Cell::from(Line::from(vec![
                    Span::styled(marker, Style::default().fg(ACCENT)),
                    Span::styled(
                        format!(" {}{}", stats.placement.name, best),
                        Style::default().fg(row_color),
                    ),
                ])),
                Cell::from(comma(stats.samples as u64)),
                Cell::from(optional_cycles(stats.median)),
                Cell::from(optional_cycles(stats.baseline)),
                Cell::from(change),
                Cell::from(Line::from(vec![
                    Span::styled("█".repeat(fill), Style::default().fg(bar_color)),
                    Span::styled("░".repeat(bar_width - fill), Style::default().fg(FAINT)),
                ])),
            ])
            .style(Style::default().fg(FG))
        });

        let title = match state.mode {
            Mode::Baseline => format!(
                " placement comparison ╱ rolling aggressor-off p50 ╱ observed spread {} ",
                state
                    .observed_spread
                    .map(|cycles| format!("{} cyc", comma_u32(cycles)))
                    .unwrap_or_else(|| "waiting".to_string())
            ),
            Mode::Contention => {
                " placement comparison ╱ rolling penalty vs each placement's own off p50 "
                    .to_string()
            }
        };
        let header = Row::new([
            "placement",
            "n",
            "cell p50",
            "off p50",
            if state.mode == Mode::Baseline {
                "Δ lowest"
            } else {
                "Δ off / penalty"
            },
            if state.mode == Mode::Baseline {
                "total cycles"
            } else {
                "penalty scale"
            },
        ])
        .style(Style::default().fg(ACCENT_ALT).add_modifier(Modifier::BOLD));
        let table = Table::new(
            rows,
            [
                Constraint::Length(24),
                Constraint::Length(8),
                Constraint::Length(14),
                Constraint::Length(14),
                Constraint::Length(20),
                Constraint::Min(10),
            ],
        )
        .header(header)
        .column_spacing(1)
        .block(panel(title, BORDER));
        frame.render_widget(table, area);
    }

    fn draw_compact_dashboard(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: Option<&ExperimentState>,
    ) {
        let title = " topology ╱ observed cell ╱ rolling p50 ";
        let Some(state) = state else {
            frame.render_widget(
                waiting_panel(
                    title,
                    "waiting for telemetry metadata and records",
                    "topology and p50 appear after valid frames",
                ),
                area,
            );
            return;
        };

        let inference = placement_name(state.inference.as_ref(), state.record.region_id);
        let dma = state
            .aggressor
            .region_id
            .map(|id| placement_name(state.dma_target.as_ref(), id))
            .unwrap_or_else(|| "off".to_string());
        let mut lines = vec![
            Line::from(Span::styled(
                format!(
                    "{} · {}",
                    mode_label(state.mode),
                    relation_message(state.relation)
                ),
                Style::default()
                    .fg(relation_color(state.relation))
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(vec![
                Span::styled("CPU/inference ► ", Style::default().fg(FAINT)),
                Span::styled(inference, Style::default().fg(ACCENT)),
                Span::styled("   GPDMA ► ", Style::default().fg(FAINT)),
                Span::styled(dma, Style::default().fg(WARNING)),
                Span::styled(
                    if state.mode == Mode::Contention {
                        format!(" / {}", footprint_label(state))
                    } else {
                        String::new()
                    },
                    Style::default().fg(DIM),
                ),
            ]),
        ];
        for region in &state.regions {
            let color = activity_color(region.activity);
            let arena = region.active_arena.as_ref().unwrap_or(&region.placement);
            lines.push(Line::from(vec![
                Span::styled(
                    if region.activity == Activity::Inactive {
                        "· "
                    } else {
                        "▎ "
                    },
                    Style::default().fg(color),
                ),
                Span::styled(
                    format!("{:<7}", region.placement.name),
                    Style::default().fg(color),
                ),
                Span::styled(
                    format!(
                        "{} + {:<8}",
                        format_address(arena.arena_addr),
                        format_arena_size(arena.arena_size)
                    ),
                    Style::default().fg(DIM),
                ),
                Span::styled(
                    format!(" [{}]", region_activity_label(region)),
                    Style::default().fg(color),
                ),
            ]));
        }
        lines.push(Line::from(vec![
            Span::styled("latest ", Style::default().fg(FAINT)),
            Span::styled(comma_u32(state.record.exec_cyc), Style::default().fg(FG)),
            separator(),
            Span::styled("off p50 ", Style::default().fg(FAINT)),
            Span::styled(
                optional_cycles(state.baseline),
                Style::default().fg(ACCENT_ALT),
            ),
            separator(),
            Span::styled(
                state
                    .penalty
                    .map(format_penalty)
                    .unwrap_or_else(|| "penalty waiting".to_string()),
                Style::default().fg(penalty_color(state.penalty)),
            ),
        ]));

        let minimum = state
            .comparisons
            .iter()
            .filter_map(|stats| stats.median)
            .min();
        lines.push(Line::from(Span::styled(
            if state.mode == Mode::Baseline {
                format!(
                    "rolling p50 · preemption possible · aggressor-off spread {}",
                    state
                        .observed_spread
                        .map(|cycles| format!("{} cyc", comma_u32(cycles)))
                        .unwrap_or_else(|| "waiting".to_string())
                )
            } else {
                "rolling p50 · wall cycles mix preemption + contention".to_string()
            },
            Style::default().fg(FAINT),
        )));
        for stats in &state.comparisons {
            let current = state.inference_bank_id == Some(stats.placement.id);
            let color = if current {
                ACCENT
            } else if stats.observed_best {
                SUCCESS
            } else {
                DIM
            };
            lines.push(Line::from(vec![
                Span::styled(
                    if current { "▎ " } else { "  " },
                    Style::default().fg(ACCENT),
                ),
                Span::styled(
                    format!("{:<7}", stats.placement.name),
                    Style::default().fg(color),
                ),
                Span::styled(
                    format!("{:>9}  ", optional_cycles(stats.median)),
                    Style::default().fg(FG),
                ),
                Span::styled(
                    comparison_change(state.mode, stats, minimum),
                    Style::default().fg(penalty_color(stats.penalty)),
                ),
                Span::styled(
                    if stats.observed_best {
                        " · observed best"
                    } else {
                        ""
                    },
                    Style::default().fg(SUCCESS),
                ),
            ]));
        }

        frame.render_widget(Paragraph::new(lines).block(panel(title, BORDER)), area);
    }

    fn draw_log(&self, frame: &mut Frame, area: Rect) {
        let height = area.height.saturating_sub(2) as usize;
        let visible = self.logs.visible(height);
        let padding = height.saturating_sub(visible.len());
        let mut lines = vec![Line::from(""); padding];
        lines.extend(visible.into_iter().map(|line| {
            let lower = line.to_ascii_lowercase();
            let color = if lower.contains("failed")
                || lower.contains("error")
                || lower.contains("rc=-")
            {
                DANGER
            } else if lower.contains("warn") || lower.contains("drop") || lower.contains("overrun")
            {
                WARNING
            } else if lower.contains("ready") || lower.contains("net rc=0") {
                SUCCESS
            } else {
                DIM
            };
            Line::from(vec![
                Span::styled("▎ ", Style::default().fg(color)),
                Span::styled(line, Style::default().fg(color)),
            ])
        }));

        frame.render_widget(
            Paragraph::new(lines).block(panel(" board log ╱ ASCII recovery ", BORDER)),
            area,
        );
    }
}

fn waiting_panel(title: &str, message: &str, detail: &str) -> Paragraph<'static> {
    Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            message.to_string(),
            Style::default().fg(WARNING).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(detail.to_string(), Style::default().fg(DIM))),
    ])
    .alignment(Alignment::Center)
    .wrap(Wrap { trim: true })
    .block(panel(title.to_string(), BORDER))
}

fn flow_line(label: &str, target: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<18}"), Style::default().fg(DIM)),
        Span::styled("────────► ", Style::default().fg(FAINT)),
        Span::styled(
            target.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ])
}

fn key_value(label: &str, value: String, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<13}"), Style::default().fg(FAINT)),
        Span::styled(value, Style::default().fg(color)),
    ])
}

fn placement_name(placement: Option<&Placement>, fallback_id: u8) -> String {
    placement
        .map(|placement| placement.name.clone())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("id{fallback_id}"))
}

fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::Baseline => "BASELINE",
        Mode::Contention => "CONTENTION",
    }
}

fn relation_label(relation: Relation) -> &'static str {
    match relation {
        Relation::Off => "AGGRESSOR OFF",
        Relation::SameBank => "SAME BANK",
        Relation::CrossBank => "CROSS BANK",
        Relation::Unknown => "RELATION UNKNOWN",
    }
}

fn relation_message(relation: Relation) -> &'static str {
    match relation {
        Relation::Off => "GPDMA AGGRESSOR OFF · BASELINE CELL",
        Relation::SameBank => "SAME BANK · SHARED SRAM TARGET",
        Relation::CrossBank => "CROSS BANK · DISTINCT SRAM TARGETS / SHARED FABRIC",
        Relation::Unknown => "RELATION UNKNOWN · METADATA INCOMPLETE",
    }
}

fn relation_color(relation: Relation) -> Color {
    match relation {
        Relation::Off => SUCCESS,
        Relation::SameBank => DANGER,
        Relation::CrossBank => ACCENT_ALT,
        Relation::Unknown => WARNING,
    }
}

fn activity_label(activity: Activity) -> &'static str {
    match activity {
        Activity::Inactive => "idle",
        Activity::Inference => "INFERENCE",
        Activity::Dma => "DMA",
        Activity::Collision => "INFERENCE + DMA",
    }
}

fn region_activity_label(region: &MemoryRegionState) -> String {
    match region
        .active_arena
        .as_ref()
        .filter(|arena| arena.id != region.placement.id)
    {
        Some(arena) => format!("{} · {}", activity_label(region.activity), arena.name),
        None => activity_label(region.activity).to_string(),
    }
}

fn activity_color(activity: Activity) -> Color {
    match activity {
        Activity::Inactive => DIM,
        Activity::Inference => ACCENT,
        Activity::Dma => WARNING,
        Activity::Collision => DANGER,
    }
}

fn footprint_label(state: &ExperimentState) -> String {
    state
        .aggressor
        .footprint_bytes
        .map(format_footprint_size)
        .unwrap_or_else(|| format!("footprint index {}", state.aggressor.footprint_index))
}

fn arena_range(placement: &Placement) -> String {
    placement
        .arena_end()
        .map(|end| {
            format!(
                "[{}, {})",
                format_address(placement.arena_addr),
                format_address(end)
            )
        })
        .unwrap_or_else(|| "invalid address range".to_string())
}

fn format_address(address: u32) -> String {
    format!("0x{address:08x}")
}

fn format_arena_size(bytes: u32) -> String {
    format!("{} B", comma(bytes as u64))
}

fn format_footprint_size(bytes: u32) -> String {
    if bytes.is_multiple_of(1024) {
        format!("{} KiB", comma((bytes / 1024) as u64))
    } else {
        format_arena_size(bytes)
    }
}

fn observed_clock(parser: &Parser) -> Option<u32> {
    parser
        .metadata
        .as_ref()
        .map(|metadata| metadata.cyccnt_hz)
        .filter(|clock| *clock != 0)
}

fn format_cycle_time(cycles: u32, clock_hz: Option<u32>) -> String {
    match clock_hz {
        Some(clock) => format!(
            "{} cyc · {:.3} ms",
            comma_u32(cycles),
            cycles as f64 * 1000.0 / clock as f64
        ),
        None => format!("{} cyc", comma_u32(cycles)),
    }
}

fn optional_cycles(cycles: Option<u32>) -> String {
    cycles
        .map(comma_u32)
        .unwrap_or_else(|| "waiting".to_string())
}

fn format_penalty(penalty: Penalty) -> String {
    let cycles = format_signed(penalty.cycles);
    match penalty.percent {
        Some(percent) => format!("{cycles} cyc / {percent:+.2}%"),
        None => format!("{cycles} cyc / percent unavailable"),
    }
}

fn format_signed(value: i64) -> String {
    if value >= 0 {
        format!("+{}", comma(value as u64))
    } else {
        format!("-{}", comma(value.unsigned_abs()))
    }
}

fn penalty_color(penalty: Option<Penalty>) -> Color {
    match penalty.map(|penalty| penalty.cycles) {
        Some(cycles) if cycles > 0 => WARNING,
        Some(cycles) if cycles < 0 => SUCCESS,
        _ => DIM,
    }
}

fn comparison_change(mode: Mode, stats: &PlacementStats, lowest: Option<u32>) -> String {
    let penalty = match mode {
        Mode::Baseline => stats
            .median
            .zip(lowest)
            .map(|(median, base)| Penalty::between(median, base)),
        Mode::Contention => stats.penalty,
    };
    penalty
        .map(format_penalty)
        .unwrap_or_else(|| "waiting".to_string())
}

impl Health {
    fn label(self) -> &'static str {
        match self {
            Self::Waiting => "waiting for bytes",
            Self::Live => "stream live",
            Self::ReplayRunning => "replay running",
            Self::TextOnly => "bytes received · no valid frame",
            Self::Rejecting => "frames arriving · validation failing",
            Self::Stale => "source stale",
            Self::ReplayDone => "replay complete",
            Self::ReplayInvalid => "replay complete · no valid frame",
            Self::RecordError => "recording failed · stream continues",
            Self::SourceError => "source error",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Live | Self::ReplayRunning | Self::ReplayDone => SUCCESS,
            Self::Waiting | Self::TextOnly | Self::Stale => WARNING,
            Self::Rejecting | Self::ReplayInvalid | Self::RecordError | Self::SourceError => DANGER,
        }
    }
}

fn main() -> ExitCode {
    match parse_args() {
        Ok(Args::Help) => {
            print_help();
            ExitCode::SUCCESS
        }
        Ok(Args::Run(options)) => match run(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("laxity-tui: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("laxity-tui: {error}\n");
            print_help();
            ExitCode::FAILURE
        }
    }
}

fn run(options: Options) -> Result<(), String> {
    if options.headless {
        return run_headless(&options);
    }
    ratatui::run(|terminal| run_tui(terminal, &options)).map_err(|error| error.to_string())
}

fn run_tui(terminal: &mut DefaultTerminal, options: &Options) -> io::Result<()> {
    let mut app = App::new(options);
    let started = Instant::now();
    let mut last_draw = Instant::now();

    while !app.should_quit {
        if options
            .duration
            .is_some_and(|duration| started.elapsed() >= duration)
        {
            break;
        }
        app.poll_source();
        let now = Instant::now();
        app.update_display_record(now, false);
        app.rate.update(now);
        app.prepare_health_effect(now);
        app.prepare_relation_effect(now);
        let elapsed = now.duration_since(last_draw);
        last_draw = now;
        terminal.draw(|frame| app.draw(frame, elapsed, now))?;

        if event::poll(Duration::from_millis(33))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press
                    && (matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                        || key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL))
                {
                    app.should_quit = true;
                }
            }
        }
    }

    app.finish_recording()
}

fn run_headless(options: &Options) -> Result<(), String> {
    if !matches!(options.source, SourceSpec::File(_)) {
        return Err("--headless requires --file".to_string());
    }

    let mut app = App::new(options);
    if let Some(error) = app.source.error() {
        return Err(format!("source open failed: {error}"));
    }
    while !app.source_finished {
        app.poll_source();
        if let Some(error) = app.source.error() {
            return Err(format!("source read failed: {error}"));
        }
    }

    app.finish_recording().map_err(|error| error.to_string())?;

    print!("{}", headless_summary(&app.parser.stats));
    if app.parser.metadata.is_none() {
        return Err("no valid header frame".to_string());
    }
    Ok(())
}

fn headless_summary(stats: &Stats) -> String {
    format!(
        "accepted_frames={}\nbatch_frames={}\ncrc_rejections={}\ndropped={}\nfalse_sync={}\nframes={}\ngaps={}\nheader_frames={}\nrecords={}\nrecords_before_header={}\nskipped_bytes={}\nwrapped={}\n",
        stats.accepted_frames,
        stats.batch_frames,
        stats.crc_rejections,
        stats.dropped,
        stats.false_sync,
        stats.frames,
        stats.gaps,
        stats.header_frames,
        stats.records,
        stats.records_before_header,
        stats.skipped_bytes,
        stats.wrapped,
    )
}

fn parse_args() -> Result<Args, String> {
    let mut args = env::args().skip(1).peekable();
    let mut source = None;
    let mut record = None;
    let mut headless = false;
    let mut duration = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Args::Help),
            "--headless" => headless = true,
            "--udp" => {
                let value = required_value(&mut args, "--udp")?;
                let port = value
                    .parse::<u16>()
                    .map_err(|_| format!("invalid UDP port: {value}"))?;
                set_source(&mut source, SourceSpec::Udp(port))?;
            }
            "--serial" => {
                let path = if args.peek().is_some_and(|value| !value.starts_with('-')) {
                    args.next().map(PathBuf::from)
                } else {
                    None
                };
                set_source(&mut source, SourceSpec::Serial(path))?;
            }
            "--file" => {
                let path = PathBuf::from(required_value(&mut args, "--file")?);
                set_source(&mut source, SourceSpec::File(path))?;
            }
            "--record" => record = Some(PathBuf::from(required_value(&mut args, "--record")?)),
            "--duration" => {
                let value = required_value(&mut args, "--duration")?;
                duration = Some(parse_duration(&value)?);
            }
            _ => return Err(format!("unknown flag: {arg}")),
        }
    }

    let source =
        source.ok_or_else(|| "choose one source: --udp, --serial or --file".to_string())?;
    if record.is_none() && !matches!(source, SourceSpec::File(_)) {
        record = Some(next_record_path());
    }
    if let (SourceSpec::File(input), Some(output)) = (&source, &record) {
        if same_path(input, output) {
            return Err("--record must not name the replay input".to_string());
        }
    }
    if duration.is_some() && matches!(source, SourceSpec::File(_)) {
        return Err("--duration is only valid for UDP or serial sources".to_string());
    }

    Ok(Args::Run(Options {
        source,
        record,
        headless,
        duration,
    }))
}

fn required_value<I>(args: &mut std::iter::Peekable<I>, flag: &str) -> Result<String, String>
where
    I: Iterator<Item = String>,
{
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn parse_duration(value: &str) -> Result<Duration, String> {
    value
        .parse::<u64>()
        .ok()
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .ok_or_else(|| format!("invalid duration in seconds: {value}"))
}

fn set_source(target: &mut Option<SourceSpec>, value: SourceSpec) -> Result<(), String> {
    if target.is_some() {
        return Err("choose exactly one source".to_string());
    }
    *target = Some(value);
    Ok(())
}

fn find_serial_port() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("LAXITY_PORT").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }

    let wanted_serial = env::var("LAXITY_STLINK_SN")
        .ok()
        .filter(|value| !value.is_empty());
    let ports = serialport::available_ports()
        .map_err(|error| format!("cannot enumerate serial ports: {error}"))?;
    select_stlink_port(ports, wanted_serial.as_deref())
}

fn select_stlink_port(
    ports: Vec<SerialPortInfo>,
    wanted_serial: Option<&str>,
) -> Result<PathBuf, String> {
    let mut candidates: Vec<_> = ports
        .into_iter()
        .filter_map(|port| {
            if !port.port_name.starts_with("/dev/cu.") {
                return None;
            }
            let SerialPortType::UsbPort(usb) = port.port_type else {
                return None;
            };
            if !usb.product.as_deref().is_some_and(is_stlink_label) {
                return None;
            }
            if wanted_serial.is_some_and(|wanted| usb.serial_number.as_deref() != Some(wanted)) {
                return None;
            }
            Some((PathBuf::from(port.port_name), usb.serial_number))
        })
        .collect();
    candidates.sort_by(|left, right| left.0.cmp(&right.0));

    match candidates.as_slice() {
        [(path, _)] => Ok(path.clone()),
        [] => match wanted_serial {
            Some(serial) => Err(format!(
                "no ST-LINK virtual COM port matched LAXITY_STLINK_SN={serial}; use --serial PATH or LAXITY_PORT"
            )),
            None => Err(
                "no ST-LINK virtual COM port found; use --serial PATH or LAXITY_PORT".to_string(),
            ),
        },
        _ => Err(format!(
            "{} ST-LINK virtual COM ports found; set LAXITY_STLINK_SN or use --serial PATH: {}",
            candidates.len(),
            candidates
                .iter()
                .map(|(path, serial)| format!(
                    "{}={}",
                    serial.as_deref().unwrap_or("unknown"),
                    path.display()
                ))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn is_stlink_label(value: &str) -> bool {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
        .contains("stlink")
}

fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn next_record_path() -> PathBuf {
    let plain = PathBuf::from("telemetry.bin");
    if !plain.exists() {
        return plain;
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    for suffix in 0..u16::MAX {
        let name = if suffix == 0 {
            format!("telemetry-{timestamp}.bin")
        } else {
            format!("telemetry-{timestamp}-{suffix}.bin")
        };
        let path = PathBuf::from(name);
        if !path.exists() {
            return path;
        }
    }
    PathBuf::from(format!("telemetry-{timestamp}-full.bin"))
}

fn panel(title: impl Into<String>, border: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .title(Span::styled(
            title.into(),
            Style::default().fg(border).add_modifier(Modifier::BOLD),
        ))
}

fn metric(value: String) -> Span<'static> {
    Span::styled(value, Style::default().fg(FG))
}

fn warning_metric(value: String) -> Span<'static> {
    let color = if value.starts_with('0') { DIM } else { WARNING };
    Span::styled(value, Style::default().fg(color))
}

fn separator() -> Span<'static> {
    Span::styled("   ·   ", Style::default().fg(FAINT))
}

fn age(instant: Option<Instant>, now: Instant) -> String {
    match instant {
        Some(instant) => {
            let elapsed = now.duration_since(instant).as_secs_f32();
            if elapsed < 10.0 {
                format!("{elapsed:.1}s")
            } else {
                format!("{elapsed:.0}s")
            }
        }
        None => "never".to_string(),
    }
}

fn format_rate(bytes_per_second: f64) -> String {
    format!("{}/s", format_bytes(bytes_per_second.round() as u64))
}

fn fit_header_pair(source: &str, record: &str, width: usize) -> (String, String) {
    let separator_width = Span::raw(RECORD_SEPARATOR).width();
    let label_width = width.saturating_sub(separator_width);
    let source_width = Span::raw(source).width();
    let record_width = Span::raw(record).width();
    if source_width + record_width <= label_width {
        return (source.to_string(), record.to_string());
    }

    let record_budget = record_width.min(label_width / 2);
    let source_budget = label_width.saturating_sub(record_budget);
    (
        elide_middle(source, source_budget),
        elide_middle(record, record_budget),
    )
}

fn elide_middle(value: &str, width: usize) -> String {
    if Span::raw(value).width() <= width {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }
    if width == 1 {
        return "…".to_string();
    }

    let left_budget = (width - 1) / 2;
    let right_budget = width - 1 - left_budget;
    let mut left = String::new();
    for character in value.chars() {
        let mut candidate = left.clone();
        candidate.push(character);
        if Span::raw(&candidate).width() > left_budget {
            break;
        }
        left.push(character);
    }

    let mut right = String::new();
    for character in value.chars().rev() {
        let mut candidate = String::with_capacity(right.len() + character.len_utf8());
        candidate.push(character);
        candidate.push_str(&right);
        if Span::raw(&candidate).width() > right_budget {
            break;
        }
        right = candidate;
    }
    format!("{left}…{right}")
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MiB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn comma(value: u64) -> String {
    let raw = value.to_string();
    let mut out = String::with_capacity(raw.len() + raw.len() / 3);
    for (index, character) in raw.chars().enumerate() {
        if index > 0 && (raw.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(character);
    }
    out
}

fn comma_u32(value: u32) -> String {
    comma(value as u64)
}

fn region_color(region_id: u8) -> Color {
    match region_id {
        1 => ACCENT,
        2 => ACCENT_ALT,
        3 => WARNING,
        5 => SUCCESS,
        6 => Color::Rgb(174, 154, 214),
        _ => FG,
    }
}

fn print_help() {
    println!(
        "laxity-tui\n\n\
         logical SRAM topology and exact-cell telemetry for the Laxity placement sweep\n\n\
         usage:\n  \
           laxity-tui --udp PORT [--record PATH] [--duration SECONDS]\n  \
           laxity-tui --serial [PATH] [--record PATH] [--duration SECONDS]\n  \
           laxity-tui --file PATH [--headless] [--record PATH]\n\n\
         sources:\n  \
           --udp PORT       listen on 0.0.0.0:PORT\n  \
           --serial [PATH]  read 921600 8N1; auto-detect one ST-LINK when omitted\n  \
           --file PATH      replay a byte-for-byte capture at demo pace\n\n\
         live control:\n  \
           --duration SEC   stop a UDP or serial session cleanly after SEC; default is no limit\n\n\
         recording:\n  \
           live sources write telemetry.bin in the launch directory and rotate if it exists\n  \
           file replay does not record unless --record is provided\n\n\
         interpretation:\n  \
           p50 retains the newest 4,096 samples per exact cell\n  \
           off p50 is this placement with aggressor_idx zero\n  \
           contention cells match the exact DMA region and footprint index\n  \
           cross bank means distinct SRAM targets on a shared fabric\n  \
           observed best is measured, not a planner decision\n  \
           exec_cyc can contain both preemption and memory contention\n\n\
         serial ownership:\n  \
           run either this TUI or capture.sh on one serial port, never both\n\n\
         keys:\n  \
           q, esc, ctrl-c   quit"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};
    use std::fs;
    use telemetry::{Record, PLACEMENT_ALT_ADDR, PLACEMENT_CONTROL};

    fn temp_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("laxity-tui-{label}-{}-{nonce}", std::process::id()))
    }

    fn test_record(seq: u32, region_id: u8, aggressor_idx: u16, exec_cyc: u32) -> Record {
        Record {
            seq,
            release_cyc: 0,
            exec_cyc,
            cpu_cyc: 0,
            stall_cyc: 0,
            model_id: 0,
            region_id,
            flags: 0,
            aggressor_idx,
            padding: 0,
            reserved: seq,
        }
    }

    fn assert_centered(buffer: &Buffer, area: Rect, needle: &str) {
        let inner_width = area.width.saturating_sub(2) as usize;
        for y in area.y + 1..area.bottom().saturating_sub(1) {
            let row: String = (area.x + 1..area.right().saturating_sub(1))
                .filter_map(|x| buffer.cell((x, y)))
                .map(|cell| cell.symbol())
                .collect();
            if let Some(left) = row.find(needle) {
                let right = inner_width.saturating_sub(left + needle.len());
                assert!(left.abs_diff(right) <= 1, "not centered: {needle:?}");
                return;
            }
        }
        panic!("missing centered text: {needle:?}");
    }

    fn usb_port(path: &str, product: &str, serial: &str) -> SerialPortInfo {
        SerialPortInfo {
            port_name: path.to_string(),
            port_type: SerialPortType::UsbPort(serialport::UsbPortInfo {
                vid: 0x0483,
                pid: 0x3754,
                serial_number: Some(serial.to_string()),
                manufacturer: Some("STMicroelectronics".to_string()),
                product: Some(product.to_string()),
            }),
        }
    }

    fn populated_app() -> App {
        let options = Options {
            source: SourceSpec::File(PathBuf::from("/missing")),
            record: None,
            headless: false,
            duration: None,
        };
        let mut app = App::new(&options);
        for (id, flags, name, address) in [
            (1, 0, "SRAM1", 0x2000_2000),
            (2, 0, "SRAM2", 0x2003_0000),
            (3, 0, "SRAM3", 0x2004_0000),
            (5, PLACEMENT_CONTROL, "SRAM1c", 0x2000_2000),
            (6, PLACEMENT_ALT_ADDR, "SRAM2b", 0x2003_9000),
        ] {
            app.parser.placements.insert(
                id,
                Placement {
                    id,
                    flags,
                    name: name.to_string(),
                    rel_cost: 1000,
                    arena_addr: address,
                    arena_size: 2944,
                },
            );
        }
        for (seq, region, cycles) in [(1, 1, 320_898), (2, 2, 320_901), (3, 3, 320_899)] {
            app.parser
                .observe_record(test_record(seq, region, 0, cycles));
        }
        for (seq, region, cycles) in [(10, 1, 338_414), (11, 2, 321_414), (12, 3, 321_535)] {
            app.parser
                .observe_record(test_record(seq, region, 0x0001, cycles));
        }
        app.display_state = Some(ExperimentState::from_parser(
            &app.parser,
            test_record(10, 1, 0x0001, 338_414),
        ));
        app
    }

    #[test]
    fn missing_source_renders_diagnostics_instead_of_a_blank_screen() {
        let options = Options {
            source: SourceSpec::File(PathBuf::from(
                "/path/that/does/not/exist/laxity-telemetry.bin",
            )),
            record: None,
            headless: false,
            duration: None,
        };
        let mut app = App::new(&options);
        let mut terminal = Terminal::new(TestBackend::new(120, 34)).unwrap();
        let now = Instant::now();
        let frame = terminal
            .draw(|frame| app.draw(frame, Duration::ZERO, now))
            .unwrap();
        let screen: String = frame
            .buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        assert!(screen.contains("source error"));
        assert!(screen.contains("source open failed"));
        assert!(screen.contains("logical SRAM topology"));
        assert!(screen.contains("placement comparison"));
        assert!(screen.contains("board log"));

        let screen_area = Rect::new(0, 0, 120, 34);
        let areas = Layout::vertical([
            Constraint::Length(7),
            Constraint::Min(12),
            Constraint::Length(5),
        ])
        .split(screen_area);
        let dashboard =
            Layout::vertical([Constraint::Min(9), Constraint::Length(9)]).split(areas[1]);
        let upper = Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)])
            .split(dashboard[0]);
        assert_centered(frame.buffer, upper[0], "waiting for placement metadata");
        assert_centered(frame.buffer, upper[1], "waiting for an observed record");
        assert_centered(
            frame.buffer,
            dashboard[1],
            "waiting for comparable experiment cells",
        );
        assert!(screen.contains("latency and arena appear after it"));
        assert!(!screen.contains("header and rec"));
    }

    #[test]
    fn compact_waiting_state_is_complete_and_centered() {
        let options = Options {
            source: SourceSpec::File(PathBuf::from("/missing")),
            record: None,
            headless: false,
            duration: None,
        };
        let mut app = App::new(&options);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let now = Instant::now();
        let frame = terminal
            .draw(|frame| app.draw(frame, Duration::ZERO, now))
            .unwrap();
        let dashboard = Rect::new(0, 7, 80, 12);

        assert_centered(
            frame.buffer,
            dashboard,
            "waiting for telemetry metadata and records",
        );
    }

    #[test]
    fn status_preserves_record_state_when_the_source_path_is_long() {
        let path = PathBuf::from(
            "/mnt/user-data/uploads/laxity/results/raw/20260909T175908Z-contention-sweep/telemetry.bin",
        );
        for (width, height) in [(80, 24), (120, 34)] {
            let options = Options {
                source: SourceSpec::File(path.clone()),
                record: None,
                headless: false,
                duration: None,
            };
            let mut app = App::new(&options);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let now = Instant::now();
            let frame = terminal
                .draw(|frame| app.draw(frame, Duration::ZERO, now))
                .unwrap();
            let screen: String = frame
                .buffer
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect();

            assert!(screen.contains("record replay only"), "width {width}");
            assert!(screen.contains('…'), "width {width}");
            assert!(screen.contains("telemetry.bin"), "width {width}");
        }
    }

    #[test]
    fn status_pair_bounds_two_long_labels() {
        let width = 48;
        let (source, record) = fit_header_pair(
            "serial /a/very/long/device/path/cu.usbmodem21303 · 921600 8N1",
            "/a/very/long/capture/path/telemetry.bin",
            width,
        );

        assert!(source.contains('…'));
        assert!(record.contains('…'));
        assert!(
            Span::raw(&source).width()
                + Span::raw(RECORD_SEPARATOR).width()
                + Span::raw(&record).width()
                <= width
        );
    }

    #[test]
    fn pending_record_becomes_visible_after_the_display_hold() {
        let mut app = populated_app();
        let before = app.display_state.as_ref().unwrap().record.seq;
        let now = Instant::now();
        app.last_display_update = Some(now);
        app.parser
            .observe_record(test_record(99, 2, 0x0103, 330_000));

        app.update_display_record(now + DISPLAY_HOLD - Duration::from_millis(1), false);
        assert_eq!(app.display_state.as_ref().unwrap().record.seq, before);

        app.update_display_record(now + DISPLAY_HOLD, false);
        assert_eq!(app.display_state.as_ref().unwrap().record.seq, 99);
    }

    #[test]
    fn zero_cycle_median_renders_without_dividing_by_zero() {
        let options = Options {
            source: SourceSpec::File(PathBuf::from("/missing")),
            record: None,
            headless: false,
            duration: None,
        };
        let mut app = App::new(&options);
        app.parser.placements.insert(
            3,
            Placement {
                id: 3,
                flags: 0,
                name: "SRAM3".to_string(),
                rel_cost: 1000,
                arena_addr: 0x2004_0000,
                arena_size: 2944,
            },
        );
        app.parser.observe_record(test_record(1, 3, 0, 0));
        app.parser.observe_record(test_record(2, 3, 3, 1));
        app.display_state = Some(ExperimentState::from_parser(
            &app.parser,
            test_record(2, 3, 3, 1),
        ));
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        let now = Instant::now();

        terminal
            .draw(|frame| app.draw(frame, Duration::ZERO, now))
            .unwrap();
    }

    #[test]
    fn measured_banks_fit_an_eighty_by_twenty_four_terminal() {
        let mut app = populated_app();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let now = Instant::now();
        let frame = terminal
            .draw(|frame| app.draw(frame, Duration::ZERO, now))
            .unwrap();
        let screen: String = frame
            .buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        for name in ["SRAM1", "SRAM2", "SRAM3"] {
            assert!(screen.contains(name), "missing {name}");
        }
        assert!(screen.contains("SAME BANK"));
        assert!(screen.contains("0x20002000"));
        assert!(screen.contains("observed best"));
        assert!(!screen.contains("SRAM1c"));
    }

    #[test]
    fn wide_layout_shows_topology_current_cell_and_comparison() {
        let mut app = populated_app();
        let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
        let now = Instant::now();
        let frame = terminal
            .draw(|frame| app.draw(frame, Duration::ZERO, now))
            .unwrap();
        let screen: String = frame
            .buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        for text in [
            "logical SRAM topology",
            "current observed cell",
            "placement comparison",
            "SAME BANK",
            "0x20002000",
            "+17,516 cyc / +5.46%",
            "observed best",
        ] {
            assert!(screen.contains(text), "missing {text}");
        }
    }

    #[test]
    fn wide_layout_threshold_keeps_the_current_cell_evidence_visible() {
        let mut app = populated_app();
        let mut terminal =
            Terminal::new(TestBackend::new(WIDE_MIN_WIDTH, WIDE_MIN_HEIGHT)).unwrap();
        let now = Instant::now();
        let frame = terminal
            .draw(|frame| app.draw(frame, Duration::ZERO, now))
            .unwrap();
        let screen: String = frame
            .buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        for text in [
            "p50 penalty",
            "DMA proof",
            "preemption + contention",
            "p50 windows retain",
        ] {
            assert!(screen.contains(text), "missing {text}");
        }
    }

    #[test]
    fn active_file_source_is_labeled_as_replay() {
        let path = temp_path("replay-health");
        fs::write(&path, b"").unwrap();
        let options = Options {
            source: SourceSpec::File(path.clone()),
            record: None,
            headless: false,
            duration: None,
        };
        let mut app = App::new(&options);
        let now = Instant::now();
        app.last_byte = Some(now);
        app.last_frame = Some(now);

        assert_eq!(app.health(now), Health::ReplayRunning);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn recorder_writes_received_bytes_exactly_once() {
        let path = temp_path("record-exact");
        let options = Options {
            source: SourceSpec::File(PathBuf::from("/missing")),
            record: Some(path.clone()),
            headless: false,
            duration: None,
        };
        let bytes = b"boot\r\nLX\x02\x01\x00\x00\xff\xff\x00";
        let mut app = App::new(&options);

        app.ingest(bytes);
        app.finish_recording().unwrap();
        drop(app);

        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn recorder_refuses_to_merge_with_an_existing_capture() {
        let path = temp_path("record-existing");
        fs::write(&path, b"existing capture").unwrap();
        let options = Options {
            source: SourceSpec::File(PathBuf::from("/missing")),
            record: Some(path.clone()),
            headless: false,
            duration: None,
        };
        let mut app = App::new(&options);

        app.ingest(b"new bytes");

        assert!(app.finish_recording().is_err());
        assert_eq!(fs::read(&path).unwrap(), b"existing capture");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn headless_summary_exposes_every_parser_counter() {
        let stats = Stats {
            frames: 6,
            accepted_frames: 1,
            header_frames: 8,
            batch_frames: 2,
            records: 9,
            dropped: 4,
            gaps: 7,
            false_sync: 5,
            crc_rejections: 3,
            skipped_bytes: 11,
            records_before_header: 10,
            wrapped: 12,
        };

        assert_eq!(
            headless_summary(&stats),
            "accepted_frames=1\nbatch_frames=2\ncrc_rejections=3\ndropped=4\nfalse_sync=5\nframes=6\ngaps=7\nheader_frames=8\nrecords=9\nrecords_before_header=10\nskipped_bytes=11\nwrapped=12\n"
        );
    }

    #[test]
    fn duration_requires_positive_whole_seconds() {
        assert_eq!(parse_duration("10").unwrap(), Duration::from_secs(10));
        assert!(parse_duration("0").is_err());
        assert!(parse_duration("1.5").is_err());
    }

    #[test]
    fn serial_selection_ignores_unrelated_modems_and_honors_stlink_serial() {
        let unrelated = usb_port("/dev/cu.usbmodem-other", "unrelated device", "OTHER");
        let first = usb_port("/dev/cu.usbmodem-a", "STM32 STLink", "AAA");
        let second = usb_port("/dev/cu.usbmodem-b", "ST-LINK V3", "BBB");

        assert_eq!(
            select_stlink_port(vec![unrelated.clone(), first.clone()], None).unwrap(),
            PathBuf::from("/dev/cu.usbmodem-a")
        );
        assert!(select_stlink_port(vec![first.clone(), second.clone()], None).is_err());
        assert_eq!(
            select_stlink_port(vec![first, second], Some("BBB")).unwrap(),
            PathBuf::from("/dev/cu.usbmodem-b")
        );
        assert!(select_stlink_port(vec![unrelated], Some("missing")).is_err());
    }

    #[test]
    fn serial_selection_uses_only_callout_devices() {
        let tty = usb_port("/dev/tty.usbmodem-a", "STLINK", "AAA");
        let callout = usb_port("/dev/cu.usbmodem-a", "STLINK", "AAA");

        assert_eq!(
            select_stlink_port(vec![tty, callout], None).unwrap(),
            PathBuf::from("/dev/cu.usbmodem-a")
        );
    }

    #[test]
    fn udp_adapter_preserves_one_datagram() {
        let socket = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        socket.set_nonblocking(true).unwrap();
        let address = socket.local_addr().unwrap();
        let sender = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        let bytes = b"status\r\nLX\x02\x01\x00\x00\xff\xff";
        sender.send_to(bytes, address).unwrap();

        let mut input = Input::Udp {
            socket,
            port: address.port(),
            peer: None,
        };
        let mut buffer = [0; 128];
        let mut received = None;
        for _ in 0..100 {
            match input.read(&mut buffer).unwrap() {
                ReadResult::Bytes(count) => {
                    received = Some(count);
                    break;
                }
                ReadResult::Empty => std::thread::sleep(Duration::from_millis(1)),
                ReadResult::Eof => unreachable!(),
            }
        }

        let count = received.expect("loopback datagram did not arrive");
        assert_eq!(&buffer[..count], bytes);
        assert!(matches!(input, Input::Udp { peer: Some(_), .. }));
    }
}
