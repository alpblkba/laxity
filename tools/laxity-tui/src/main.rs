mod telemetry;

use std::{
    collections::{BTreeSet, VecDeque},
    env,
    fs::{self, File, OpenOptions},
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
    widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table},
    DefaultTerminal, Frame,
};
use serialport::{DataBits, FlowControl, Parity, SerialPort, StopBits};
use tachyonfx::{fx, EffectManager, Interpolation};

use telemetry::{ParseDelta, Parser};

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
    last_health: Option<Health>,
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
            last_health: None,
        }
    }

    fn poll_source(&mut self) {
        if self.source_finished || self.source.error().is_some() {
            return;
        }

        let pace_replay = self.source.is_file();
        let mut buffer = [0u8; READ_SIZE];
        for _ in 0..MAX_READS_PER_TICK {
            match self.source.read(&mut buffer) {
                Ok(ReadResult::Bytes(count)) => {
                    self.ingest(&buffer[..count]);
                    if pace_replay {
                        break;
                    }
                }
                Ok(ReadResult::Empty) => break,
                Ok(ReadResult::Eof) => {
                    let delta = self.parser.finish();
                    self.accept_delta(delta, Instant::now());
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
            return Health::Live;
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

    fn draw(&mut self, frame: &mut Frame, elapsed: Duration, now: Instant) {
        let screen = frame.area();
        let areas = Layout::vertical([
            Constraint::Length(7),
            Constraint::Min(8),
            Constraint::Length(9),
        ])
        .split(screen);
        self.header_area = areas[0];

        self.draw_status(frame, areas[0], now);
        self.draw_regions(frame, areas[1]);
        self.draw_log(frame, areas[2]);
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
                Span::styled(self.source.label(), Style::default().fg(ACCENT_ALT)),
                Span::styled("   ·   record ", Style::default().fg(FAINT)),
                Span::styled(record_label, Style::default().fg(record_style)),
            ])
            .alignment(Alignment::Center),
            Line::from(vec![
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
            Paragraph::new(lines).block(panel(" laxity ╱ live telemetry ", health.color())),
            area,
        );
    }

    fn draw_regions(&self, frame: &mut Frame, area: Rect) {
        let title = " observed wall cycles · preemption + contention are not separated ";
        let mut ids: BTreeSet<u8> = self.parser.placements.keys().copied().collect();
        ids.extend(self.parser.region_exec.keys().copied());

        if ids.is_empty() {
            let text = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "waiting for a valid header and telemetry records",
                    Style::default().fg(WARNING).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "binary frames will become per-region medians here",
                    Style::default().fg(DIM),
                )),
            ];
            frame.render_widget(
                Paragraph::new(text)
                    .alignment(Alignment::Center)
                    .block(panel(title, BORDER)),
                area,
            );
            return;
        }

        let max_median = ids
            .iter()
            .filter_map(|id| self.parser.median_exec(*id))
            .max()
            .unwrap_or(1)
            .max(1);
        let latest_region = self.parser.latest_record.map(|record| record.region_id);
        let clock_hz = self
            .parser
            .metadata
            .as_ref()
            .map(|meta| meta.cyccnt_hz)
            .filter(|clock| *clock != 0)
            .unwrap_or(160_000_000);
        let bar_width = area.width.saturating_sub(63).clamp(8, 28) as usize;

        let rows = ids.into_iter().map(|id| {
            let values = self.parser.region_exec.get(&id);
            let count = values.map_or(0, Vec::len);
            let median = self.parser.median_exec(id);
            let name = self
                .parser
                .placements
                .get(&id)
                .map(|placement| placement.name.as_str())
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("id{id}"));
            let filled = median
                .map(|value| ((value as u64 * bar_width as u64) / max_median as u64) as usize)
                .unwrap_or(0)
                .min(bar_width);
            let marker = if latest_region == Some(id) {
                "▎"
            } else {
                " "
            };
            let color = region_color(id);
            Row::new(vec![
                Cell::from(Line::from(vec![
                    Span::styled(marker, Style::default().fg(ACCENT)),
                    Span::styled(format!(" {name} [{id}]"), Style::default().fg(color)),
                ])),
                Cell::from(comma(count as u64)),
                Cell::from(
                    median
                        .map(comma_u32)
                        .unwrap_or_else(|| "waiting".to_string()),
                ),
                Cell::from(
                    median
                        .map(|value| format!("{:.3} ms", value as f64 * 1000.0 / clock_hz as f64))
                        .unwrap_or_else(|| "·".to_string()),
                ),
                Cell::from(Line::from(vec![
                    Span::styled("█".repeat(filled), Style::default().fg(color)),
                    Span::styled("░".repeat(bar_width - filled), Style::default().fg(FAINT)),
                ])),
            ])
            .style(Style::default().fg(FG))
            .height(1)
        });

        let header = Row::new([
            "region",
            "samples",
            "median exec_cyc",
            "wall time",
            "observed load",
        ])
        .style(Style::default().fg(ACCENT_ALT).add_modifier(Modifier::BOLD));
        let table = Table::new(
            rows,
            [
                Constraint::Length(18),
                Constraint::Length(10),
                Constraint::Length(17),
                Constraint::Length(12),
                Constraint::Min(13),
            ],
        )
        .header(header)
        .column_spacing(1)
        .block(panel(title, BORDER));
        frame.render_widget(table, area);
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

impl Health {
    fn label(self) -> &'static str {
        match self {
            Self::Waiting => "waiting for bytes",
            Self::Live => "stream live",
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
            Self::Live | Self::ReplayDone => SUCCESS,
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
    let mut last_draw = Instant::now();

    while !app.should_quit {
        app.poll_source();
        let now = Instant::now();
        app.rate.update(now);
        app.prepare_health_effect(now);
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

    println!("frames={}", app.parser.stats.frames);
    println!("records={}", app.parser.stats.records);
    println!("dropped={}", app.parser.stats.dropped);
    println!("gaps={}", app.parser.stats.gaps);
    println!("false_sync={}", app.parser.stats.false_sync);
    if app.parser.metadata.is_none() {
        return Err("no valid header frame".to_string());
    }
    Ok(())
}

fn parse_args() -> Result<Args, String> {
    let mut args = env::args().skip(1).peekable();
    let mut source = None;
    let mut record = None;
    let mut headless = false;

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

    Ok(Args::Run(Options {
        source,
        record,
        headless,
    }))
}

fn required_value<I>(args: &mut std::iter::Peekable<I>, flag: &str) -> Result<String, String>
where
    I: Iterator<Item = String>,
{
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn set_source(target: &mut Option<SourceSpec>, value: SourceSpec) -> Result<(), String> {
    if target.is_some() {
        return Err("choose exactly one source".to_string());
    }
    *target = Some(value);
    Ok(())
}

fn find_serial_port() -> Result<PathBuf, String> {
    let mut ports: Vec<_> = fs::read_dir("/dev")
        .map_err(|error| format!("cannot scan /dev: {error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("cu.usbmodem"))
        })
        .collect();
    ports.sort();
    match ports.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err("no /dev/cu.usbmodem* serial port found".to_string()),
        _ => Err(format!(
            "multiple serial ports found: {}",
            ports
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
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

fn panel(title: &'static str, border: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .title(Span::styled(
            title,
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
         usage:\n  \
           laxity-tui --udp PORT [--record PATH]\n  \
           laxity-tui --serial [PATH] [--record PATH]\n  \
           laxity-tui --file PATH [--headless] [--record PATH]\n\n\
         sources:\n  \
           --udp PORT       listen on 0.0.0.0:PORT\n  \
           --serial [PATH]  read 921600 8N1; auto-detect one usbmodem when omitted\n  \
           --file PATH      replay a byte-for-byte capture\n\n\
         recording:\n  \
           live sources write a new telemetry.bin and rotate the name if it exists\n  \
           file replay does not record unless --record is provided\n\n\
         keys:\n  \
           q, esc, ctrl-c   quit"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn temp_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("laxity-tui-{label}-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn missing_source_renders_diagnostics_instead_of_a_blank_screen() {
        let options = Options {
            source: SourceSpec::File(PathBuf::from(
                "/path/that/does/not/exist/laxity-telemetry.bin",
            )),
            record: None,
            headless: false,
        };
        let mut app = App::new(&options);
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
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
        assert!(screen.contains("observed wall cycles"));
        assert!(screen.contains("board log"));
    }

    #[test]
    fn zero_cycle_median_renders_without_dividing_by_zero() {
        let options = Options {
            source: SourceSpec::File(PathBuf::from("/missing")),
            record: None,
            headless: false,
        };
        let mut app = App::new(&options);
        app.parser.region_exec.insert(3, vec![0]);
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        let now = Instant::now();

        terminal
            .draw(|frame| app.draw(frame, Duration::ZERO, now))
            .unwrap();
    }

    #[test]
    fn five_regions_fit_an_eighty_by_twenty_four_terminal() {
        let options = Options {
            source: SourceSpec::File(PathBuf::from("/missing")),
            record: None,
            headless: false,
        };
        let mut app = App::new(&options);
        for (id, name) in [
            (1, "SRAM1"),
            (2, "SRAM2"),
            (3, "SRAM3"),
            (5, "SRAM1c"),
            (6, "SRAM2b"),
        ] {
            app.parser.placements.insert(
                id,
                telemetry::Placement {
                    id,
                    name: name.to_string(),
                    rel_cost: 1000,
                    arena_addr: 0,
                    arena_size: 303_104,
                },
            );
            app.parser.region_exec.insert(id, vec![320_000]);
        }

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

        for name in ["SRAM1", "SRAM2", "SRAM3", "SRAM1c", "SRAM2b"] {
            assert!(screen.contains(name), "missing {name}");
        }
        assert!(screen.contains("observed load"));
    }

    #[test]
    fn recorder_writes_received_bytes_exactly_once() {
        let path = temp_path("record-exact");
        let options = Options {
            source: SourceSpec::File(PathBuf::from("/missing")),
            record: Some(path.clone()),
            headless: false,
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
        };
        let mut app = App::new(&options);

        app.ingest(b"new bytes");

        assert!(app.finish_recording().is_err());
        assert_eq!(fs::read(&path).unwrap(), b"existing capture");
        fs::remove_file(path).unwrap();
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
