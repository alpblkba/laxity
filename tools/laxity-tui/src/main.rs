mod experiment;
mod lx_v2;
mod measurement;
mod model;
mod profile;
mod scene;
mod telemetry;

use std::{
    collections::{BTreeMap, VecDeque},
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
use lx_v2::LxV2Adapter;
use measurement::{MeasurementStore, SAMPLE_LIMIT};
use model::{Device, DeviceState, MemoryRegionId, Placement};
use scene::{
    transition_effects, EffectKind, EffectTarget, MemorySceneModel, RegionScene, SemanticEffect,
    ViewMetric,
};
use telemetry::{LxEvent, ParseDelta, Stats, TelemetryState};

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
const WIDE_MIN_WIDTH: u16 = 110;
const WIDE_MIN_HEIGHT: u16 = 34;
const PROFILE_SEPARATOR: &str = " · profile ";
const SIMULATED_SEPARATOR: &str = " · ";
const SIMULATED_LABEL: &str = "SIMULATED";
const SIMULATED_RECORD_SEPARATOR: &str = " · ";
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
    profile: Option<PathBuf>,
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
    telemetry: TelemetryState,
    adapter: LxV2Adapter,
    device: DeviceState,
    measurements: MeasurementStore,
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
    display_revision: u64,
    last_display_update: Option<Instant>,
    view: View,
    metric: ViewMetric,
    selected_region: usize,
    semantic_effects: Vec<SemanticEffect>,
    region_areas: BTreeMap<MemoryRegionId, Rect>,
    memory_scene_key: Option<(u64, Option<u64>, ViewMetric, Option<MemoryRegionId>)>,
    memory_scene: Option<MemorySceneModel>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum View {
    #[default]
    Telemetry,
    Memory,
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

    fn concise_label(&self) -> String {
        match self {
            Self::Udp { port, .. } => format!("udp 0.0.0.0:{port}"),
            Self::Serial { path, .. } => format!("serial {}", path.display()),
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
    fn with_device(options: &Options, device: Device) -> Self {
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
            telemetry: TelemetryState::default(),
            adapter: LxV2Adapter::default(),
            device: DeviceState::new(device),
            measurements: MeasurementStore::default(),
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
            display_revision: 0,
            last_display_update: None,
            view: View::Telemetry,
            metric: ViewMetric::P50,
            selected_region: 0,
            semantic_effects: Vec::new(),
            region_areas: BTreeMap::new(),
            memory_scene_key: None,
            memory_scene: None,
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
                    let delta = self.telemetry.finish();
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
        let delta = self.telemetry.feed(bytes);
        self.accept_delta(delta, now);
    }

    fn update_display_record(&mut self, now: Instant, force: bool) {
        let Some(latest) = self.measurements.latest().cloned() else {
            return;
        };
        if self
            .display_state
            .as_ref()
            .map(|state| state.measurement.sequence)
            == Some(latest.sequence)
            && self.display_revision == self.device.topology_revision
        {
            return;
        }
        let ready = self
            .last_display_update
            .is_none_or(|last| now.duration_since(last) >= DISPLAY_HOLD);
        if force || ready {
            let next = ExperimentState::from_model(&self.device, &self.measurements, latest);
            self.semantic_effects
                .extend(transition_effects(self.display_state.as_ref(), &next));
            self.display_state = Some(next);
            self.display_revision = self.device.topology_revision;
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
            self.semantic_effects.push(SemanticEffect {
                target: EffectTarget::TransportHeader,
                kind: EffectKind::Warning,
            });
        }
        for event in delta.events {
            match event {
                LxEvent::Header(header) => {
                    if self.adapter.apply_header(&mut self.device, header) {
                        self.measurements
                            .begin_revision(self.device.topology_revision);
                        self.display_state = None;
                    }
                }
                LxEvent::Record(record) => {
                    let measurement = self.adapter.adapt_record(&self.device, record);
                    self.measurements.observe(measurement);
                }
            }
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
            return if self.telemetry.stats.accepted_frames > 0 {
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
            let kind = match health {
                Health::SourceError | Health::RecordError | Health::ReplayInvalid => {
                    EffectKind::SourceError
                }
                Health::Rejecting | Health::Stale | Health::TextOnly => EffectKind::Warning,
                _ => EffectKind::Arrival,
            };
            self.semantic_effects.push(SemanticEffect {
                target: EffectTarget::TransportHeader,
                kind,
            });
        }
        self.last_health = Some(health);
    }

    fn prepare_semantic_effects(&mut self) {
        for request in std::mem::take(&mut self.semantic_effects) {
            let area = match &request.target {
                EffectTarget::TransportHeader => self.header_area,
                EffectTarget::MemoryRegion(region_id) => self
                    .region_areas
                    .get(region_id)
                    .copied()
                    .unwrap_or(self.memory_area),
                EffectTarget::Placement(placement_id) => self
                    .device
                    .placements
                    .get(placement_id)
                    .and_then(|placement| placement.region_id.as_ref())
                    .and_then(|region_id| self.region_areas.get(region_id))
                    .copied()
                    .unwrap_or(self.memory_area),
                EffectTarget::RequesterPath(_, region_id) => region_id
                    .as_ref()
                    .and_then(|region_id| self.region_areas.get(region_id))
                    .copied()
                    .unwrap_or(self.memory_area),
            };
            if area.is_empty() {
                continue;
            }
            let (color, duration) = match request.kind {
                EffectKind::Arrival => (ACCENT_ALT, 100),
                EffectKind::RelationChange => (WARNING, 220),
                EffectKind::Warning => (DANGER, 180),
                EffectKind::SourceError => (DANGER, 240),
            };
            self.effects.add_effect(
                fx::fade_from_fg(color, (duration, Interpolation::SineOut)).with_area(area),
            );
        }
    }

    fn draw(&mut self, frame: &mut Frame, elapsed: Duration, now: Instant) {
        let screen = frame.area();
        self.region_areas.clear();
        if self.view == View::Memory {
            self.draw_memory_view(frame, screen, now);
            self.effects
                .process_effects(elapsed.into(), frame.buffer_mut(), screen);
            return;
        }
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

    fn draw_memory_view(&mut self, frame: &mut Frame, screen: Rect, now: Instant) {
        let areas = Layout::vertical([
            Constraint::Length(7),
            Constraint::Min(10),
            Constraint::Length(4),
        ])
        .split(screen);
        self.header_area = areas[0];
        self.memory_area = areas[1];
        self.draw_status(frame, areas[0], now);

        let region_count = self.device.device.memory_regions.len();
        if region_count > 0 {
            self.selected_region %= region_count;
        }
        let selected = self
            .device
            .device
            .memory_regions
            .keys()
            .nth(self.selected_region)
            .cloned();
        let scene_key = (
            self.device.topology_revision,
            self.display_state
                .as_ref()
                .map(|state| state.measurement.sequence),
            self.metric,
            selected.clone(),
        );
        if self.memory_scene_key.as_ref() != Some(&scene_key) {
            self.memory_scene = Some(MemorySceneModel::build(
                &self.device,
                &self.measurements,
                self.display_state.as_ref(),
                self.metric,
                selected.as_ref(),
            ));
            self.memory_scene_key = Some(scene_key);
        }
        let scene = self
            .memory_scene
            .as_ref()
            .expect("a memory scene is built before rendering");
        Self::draw_memory_scene(frame, areas[1], scene, &mut self.region_areas);
        Self::draw_memory_footer(frame, areas[2], scene);
    }

    fn draw_memory_scene(
        frame: &mut Frame,
        area: Rect,
        scene: &MemorySceneModel,
        region_areas: &mut BTreeMap<MemoryRegionId, Rect>,
    ) {
        let title = format!(
            " logical address slabs ╱ {} ╱ {} ",
            scene.metric.label(),
            scene.device_label
        );
        let block = panel(title, BORDER);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if scene.regions.is_empty() {
            frame.render_widget(
                Paragraph::new("device profile declares no memory regions")
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(WARNING)),
                inner,
            );
            return;
        }

        let max_size = scene
            .regions
            .iter()
            .map(|region| region.range.size)
            .max()
            .unwrap_or(1)
            .max(1);
        let max_bar_width = inner.width.saturating_sub(5).clamp(8, 72) as usize;
        let values: Vec<_> = scene
            .regions
            .iter()
            .flat_map(|region| {
                region
                    .placements
                    .iter()
                    .filter_map(|placement| placement.metric_value)
                    .chain(region.metric_value)
            })
            .collect();
        let value_max = values.iter().copied().map(f64::abs).fold(0.0_f64, f64::max);
        let compact = inner.height < scene.regions.len() as u16 * 3;
        let stride = if compact { 2 } else { 3 };
        let mut lines = Vec::new();

        for (index, region) in scene.regions.iter().enumerate() {
            let y = inner.y.saturating_add(lines.len() as u16);
            if y >= inner.bottom() {
                break;
            }
            let color = if region.selected {
                SUCCESS
            } else if region.activity == Activity::Inactive {
                region_color(index)
            } else {
                activity_color(region.activity)
            };
            region_areas.insert(
                region.id.clone(),
                Rect::new(inner.x, y, inner.width, stride.min(inner.bottom() - y)),
            );
            let metric = region
                .metric_value
                .map(|value| format!("{} {:.1}", scene.metric.label(), value))
                .unwrap_or_else(|| format!("{} unavailable", scene.metric.label()));
            lines.push(Line::from(vec![
                Span::styled(
                    if region.selected { "▎ " } else { "· " },
                    Style::default().fg(color),
                ),
                Span::styled(
                    format!("{:<8}", region.label),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "{}..{}  ",
                        format_address(region.range.start),
                        region
                            .range
                            .end()
                            .map(format_address)
                            .unwrap_or_else(|| "invalid".to_string())
                    ),
                    Style::default().fg(DIM),
                ),
                Span::styled(
                    metric,
                    Style::default().fg(metric_color(scene.metric, region.metric_value)),
                ),
            ]));

            let width = ((region.range.size.saturating_mul(max_bar_width as u64) / max_size)
                as usize)
                .clamp(4, max_bar_width);
            lines.push(memory_slab_line(region, width, value_max, color));
            if !compact {
                let placement_text = if region.placements.is_empty() {
                    "no measured placement ranges".to_string()
                } else {
                    region
                        .placements
                        .iter()
                        .map(|placement| {
                            let value = placement
                                .metric_value
                                .map(|value| format!(" · {} {value:.1}", scene.metric.label()))
                                .unwrap_or_default();
                            format!(
                                "{} {}{value}",
                                placement.label,
                                format_range(placement.range.start, placement.range.end())
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" · ")
                };
                lines.push(Line::from(Span::styled(
                    format!("    {placement_text}"),
                    Style::default().fg(FAINT),
                )));
            }
        }

        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn draw_memory_footer(frame: &mut Frame, area: Rect, scene: &MemorySceneModel) {
        let selected = scene.regions.iter().find(|region| region.selected);
        let first = selected
            .map(|region| {
                format!(
                    "selected {} · {} · {} placement range(s)",
                    region.label,
                    format_range(region.range.start, region.range.end()),
                    region.placements.len()
                )
            })
            .unwrap_or_else(|| "no memory region selected".to_string());
        let flow = scene.flows.first().map(|flow| {
            let target = flow
                .target_region_id
                .as_ref()
                .and_then(|id| scene.regions.iter().find(|region| &region.id == id))
                .map(|region| region.label.as_str())
                .unwrap_or("unknown target");
            format!("   ·   {} ───► {target}", flow.label)
        });
        let topology = if scene.physical_topology_available {
            "profile declares verified physical topology"
        } else {
            "logical address view · physical interconnect unavailable"
        };
        let lines = vec![
            Line::from(vec![
                Span::styled(first, Style::default().fg(FG)),
                Span::styled(flow.unwrap_or_default(), Style::default().fg(WARNING)),
            ]),
            Line::from(vec![
                Span::styled(topology, Style::default().fg(FAINT)),
                Span::styled(
                    "   ·   ↑/↓ select   tab metric   m dashboard   q quit",
                    Style::default().fg(DIM),
                ),
            ]),
        ];
        frame.render_widget(
            Paragraph::new(lines).block(panel(" memory view controls ", BORDER)),
            area,
        );
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
        let simulated_profile = self
            .adapter
            .metadata()
            .filter(|metadata| metadata.simulated)
            .map(|_| self.device.device.id.to_string());
        let simulated = simulated_profile.is_some();
        let source = if simulated && area.width < WIDE_MIN_WIDTH {
            self.source.concise_label()
        } else {
            self.source.label()
        };
        let (source_label, profile_label, record_label) = fit_status_labels(
            &source,
            simulated_profile.as_deref(),
            &record_label,
            area.width.saturating_sub(2) as usize,
        );
        let clock = self
            .adapter
            .metadata()
            .as_ref()
            .map(|metadata| format!("{:.3} MHz CYCCNT", metadata.cyccnt_hz as f64 / 1_000_000.0))
            .unwrap_or_else(|| "clock waiting".to_string());
        let mut source_line = vec![Span::styled(source_label, Style::default().fg(ACCENT_ALT))];
        if let Some(profile_label) = profile_label {
            source_line.push(Span::styled(PROFILE_SEPARATOR, Style::default().fg(FAINT)));
            source_line.push(Span::styled(profile_label, Style::default().fg(WARNING)));
            source_line.push(Span::styled(
                SIMULATED_SEPARATOR,
                Style::default().fg(FAINT),
            ));
            source_line.push(Span::styled(
                SIMULATED_LABEL,
                Style::default()
                    .fg(Color::Black)
                    .bg(DANGER)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        source_line.push(Span::styled(
            if simulated {
                SIMULATED_RECORD_SEPARATOR
            } else {
                RECORD_SEPARATOR
            },
            Style::default().fg(FAINT),
        ));
        source_line.push(Span::styled(
            record_label,
            Style::default().fg(record_style),
        ));

        let lines = vec![
            Line::from(vec![
                Span::styled("● ", Style::default().fg(health.color())),
                Span::styled(
                    health.label(),
                    Style::default()
                        .fg(health.color())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("   ·   m view   ·   q quit", Style::default().fg(FAINT)),
            ])
            .alignment(Alignment::Center),
            Line::from(source_line).alignment(Alignment::Center),
            Line::from(vec![
                metric(clock),
                separator(),
                metric(format_rate(self.rate.bytes_per_second)),
                separator(),
                metric(format!(
                    "{} frames",
                    comma(self.telemetry.stats.accepted_frames)
                )),
                separator(),
                metric(format!("{} records", comma(self.telemetry.stats.records))),
            ])
            .alignment(Alignment::Center),
            Line::from(vec![
                warning_metric(format!(
                    "{} crc rejects",
                    comma(self.telemetry.stats.crc_rejections)
                )),
                separator(),
                warning_metric(format!("{} seq gaps", comma(self.telemetry.stats.gaps))),
                separator(),
                warning_metric(format!(
                    "{} target drops",
                    comma(self.telemetry.stats.dropped as u64)
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
        let title = " logical memory topology ╱ traffic flow ";
        let Some(state) = state else {
            frame.render_widget(
                waiting_panel(
                    title,
                    "waiting for placement metadata",
                    "regions appear after profile and placement metadata",
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
                        "placement {} · containing region unavailable",
                        state.measurement.placement_id
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

        let inference = placement_name(
            state.inference.as_ref(),
            state.measurement.placement_id.to_string(),
        );
        let requester = state
            .measurement
            .requester
            .as_ref()
            .map(|load| {
                placement_name(
                    state.requester_target.as_ref(),
                    load.target_placement_id
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "unknown".to_string()),
                )
            })
            .unwrap_or_else(|| "off".to_string());
        let mut lines = vec![
            flow_line("workload", &inference, ACCENT),
            flow_line(
                state.requester_label.as_deref().unwrap_or("requester"),
                &requester,
                WARNING,
            ),
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
            let arena = region
                .active_placement
                .as_ref()
                .unwrap_or(&region.placement);
            let activity = region_activity_label(region);
            let marker = if region.activity == Activity::Inactive {
                "·"
            } else {
                "▎"
            };
            if detailed {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{marker} {:<8}", region.region.label),
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
                        format_arena_size(arena.range.size),
                        comma(region.samples as u64)
                    ),
                    Style::default().fg(DIM),
                )));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{marker} {:<7}", region.region.label),
                        Style::default().fg(color),
                    ),
                    Span::styled(
                        format!(
                            " {} + {} ",
                            format_address(arena.range.start),
                            format_arena_size(arena.range.size)
                        ),
                        Style::default().fg(DIM),
                    ),
                    Span::styled(format!("[{activity}]"), Style::default().fg(color)),
                ]));
            }
        }
        lines.push(
            Line::from(Span::styled(
                "logical regions from device profile · placements are measured ranges",
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

        let clock_hz = observed_clock(&self.adapter);
        let inference = placement_name(
            state.inference.as_ref(),
            state.measurement.placement_id.to_string(),
        );
        let arena = state
            .inference
            .as_ref()
            .map(|placement| {
                format!(
                    "{} + {}",
                    format_address(placement.range.start),
                    format_arena_size(placement.range.size)
                )
            })
            .unwrap_or_else(|| "metadata unavailable".to_string());
        let requester = state
            .measurement
            .requester
            .as_ref()
            .map(|load| {
                format!(
                    "{} · {}",
                    placement_name(
                        state.requester_target.as_ref(),
                        load.target_placement_id
                            .as_ref()
                            .map(ToString::to_string)
                            .unwrap_or_else(|| "unknown".to_string()),
                    ),
                    footprint_label(state)
                )
            })
            .unwrap_or_else(|| "off".to_string());
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
            key_value("requester", requester, WARNING),
            key_value(
                "latest",
                state
                    .measurement
                    .metrics
                    .execution_cycles
                    .map(|cycles| format_cycle_time(cycles, clock_hz))
                    .unwrap_or_else(|| "unavailable".to_string()),
                FG,
            ),
            key_value(
                "off p50",
                optional_cycles(state.baseline.map(|statistics| statistics.p50)),
                ACCENT_ALT,
            ),
            key_value(
                "cell p50",
                state
                    .current
                    .map(|statistics| {
                        format!(
                            "{} · n {}",
                            comma(statistics.p50),
                            comma(statistics.count as u64)
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
            let progress = state
                .measurement
                .metrics
                .requester_progress
                .unwrap_or_default();
            let proof = match state.requester_advanced {
                Some(true) => format!("advanced · counter {}", comma(progress)),
                Some(false) => format!("pending · counter {}", comma(progress)),
                None => "unavailable".to_string(),
            };
            lines.push(key_value("requester proof", proof, DIM));
        }
        lines.push(Line::from(Span::styled(
            "wall cycles mix preemption + contention",
            Style::default().fg(FAINT),
        )));
        lines.push(Line::from(Span::styled(
            format!(
                "p50 windows retain ≤ {} samples/cell",
                comma(SAMPLE_LIMIT as u64)
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
            .filter_map(PlacementStats::p50)
            .min();
        let maximum = state
            .comparisons
            .iter()
            .filter_map(PlacementStats::p50)
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

        let rows = state.comparisons.iter().enumerate().map(|(index, stats)| {
            let current = state.measurement.placement_id == stats.placement.id;
            let marker = if current { "▎" } else { " " };
            let best = if stats.observed_best {
                " · observed best"
            } else {
                ""
            };
            let change = comparison_change(state.mode, stats, minimum);
            let fill = match (state.mode, stats.p50(), stats.penalty) {
                (Mode::Baseline, Some(median), _) => {
                    (median * bar_width as u64 / max_median) as usize
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
                region_color(index)
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
                        format!(" {}{}", placement_label(&stats.placement), best),
                        Style::default().fg(row_color),
                    ),
                ])),
                Cell::from(comma(stats.samples() as u64)),
                Cell::from(optional_cycles(stats.p50())),
                Cell::from(optional_cycles(
                    stats.baseline.map(|statistics| statistics.p50),
                )),
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
                    .map(|cycles| format!("{} cyc", comma(cycles)))
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

        let inference = placement_name(
            state.inference.as_ref(),
            state.measurement.placement_id.to_string(),
        );
        let requester = state
            .measurement
            .requester
            .as_ref()
            .map(|load| {
                placement_name(
                    state.requester_target.as_ref(),
                    load.target_placement_id
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "unknown".to_string()),
                )
            })
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
                Span::styled("workload ► ", Style::default().fg(FAINT)),
                Span::styled(inference, Style::default().fg(ACCENT)),
                Span::styled(
                    format!(
                        "   {} ► ",
                        state.requester_label.as_deref().unwrap_or("requester")
                    ),
                    Style::default().fg(FAINT),
                ),
                Span::styled(requester, Style::default().fg(WARNING)),
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
            let arena = region
                .active_placement
                .as_ref()
                .unwrap_or(&region.placement);
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
                    format!("{:<7}", region.region.label),
                    Style::default().fg(color),
                ),
                Span::styled(
                    format!(
                        "{} + {:<8}",
                        format_address(arena.range.start),
                        format_arena_size(arena.range.size)
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
            Span::styled(
                state
                    .measurement
                    .metrics
                    .execution_cycles
                    .map(comma)
                    .unwrap_or_else(|| "unavailable".to_string()),
                Style::default().fg(FG),
            ),
            separator(),
            Span::styled("off p50 ", Style::default().fg(FAINT)),
            Span::styled(
                optional_cycles(state.baseline.map(|statistics| statistics.p50)),
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
            .filter_map(PlacementStats::p50)
            .min();
        lines.push(Line::from(Span::styled(
            if state.mode == Mode::Baseline {
                format!(
                    "rolling p50 · preemption possible · aggressor-off spread {}",
                    state
                        .observed_spread
                        .map(|cycles| format!("{} cyc", comma(cycles)))
                        .unwrap_or_else(|| "waiting".to_string())
                )
            } else {
                "rolling p50 · wall cycles mix preemption + contention".to_string()
            },
            Style::default().fg(FAINT),
        )));
        for stats in &state.comparisons {
            let current = state.measurement.placement_id == stats.placement.id;
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
                    format!("{:<7}", placement_label(&stats.placement)),
                    Style::default().fg(color),
                ),
                Span::styled(
                    format!("{:>9}  ", optional_cycles(stats.p50())),
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

fn memory_slab_line(
    region: &RegionScene,
    width: usize,
    value_max: f64,
    color: Color,
) -> Line<'static> {
    let mut cells = vec![("░", Style::default().fg(FAINT)); width];
    for placement in &region.placements {
        if !region.range.contains(placement.range) {
            continue;
        }
        let offset = placement.range.start - region.range.start;
        let start = (offset.saturating_mul(width as u64) / region.range.size) as usize;
        let end_offset = offset.saturating_add(placement.range.size);
        let end = end_offset
            .saturating_mul(width as u64)
            .div_ceil(region.range.size) as usize;
        let intensity = placement
            .metric_value
            .map(|value| {
                if value_max <= f64::EPSILON {
                    0
                } else {
                    (value.abs() / value_max * 3.0).round() as usize
                }
            })
            .unwrap_or(0)
            .min(3);
        let symbol = ["░", "▒", "▓", "█"][intensity];
        let placement_style = Style::default().fg(if placement.active { ACCENT } else { color });
        for cell in cells
            .iter_mut()
            .take(end.clamp(start + 1, width))
            .skip(start.min(width.saturating_sub(1)))
        {
            *cell = (symbol, placement_style);
        }
    }

    let mut spans = Vec::with_capacity(width + 3);
    spans.push(Span::styled("   ╱", Style::default().fg(color)));
    spans.extend(
        cells
            .into_iter()
            .map(|(symbol, style)| Span::styled(symbol, style)),
    );
    spans.push(Span::styled("╱│", Style::default().fg(color)));
    Line::from(spans)
}

fn metric_color(metric: ViewMetric, value: Option<f64>) -> Color {
    match (metric, value) {
        (_, None) => FAINT,
        (ViewMetric::Slack, Some(value)) if value < 0.0 => DANGER,
        (ViewMetric::Slack, Some(_)) => SUCCESS,
        (ViewMetric::Penalty, Some(value)) if value < 0.0 => SUCCESS,
        (ViewMetric::Penalty, Some(value)) if value > 0.0 => WARNING,
        (ViewMetric::Penalty, Some(_)) => DIM,
        (_, Some(_)) => ACCENT_ALT,
    }
}

fn format_range(start: u64, end: Option<u64>) -> String {
    end.map(|end| format!("[{}, {})", format_address(start), format_address(end)))
        .unwrap_or_else(|| "invalid address range".to_string())
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

fn placement_name(placement: Option<&Placement>, fallback_id: String) -> String {
    placement
        .and_then(|placement| placement.label.clone())
        .filter(|name| !name.is_empty())
        .unwrap_or(fallback_id)
}

fn placement_label(placement: &Placement) -> String {
    placement_name(Some(placement), placement.id.to_string())
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
        Relation::SameRegion => "SAME REGION",
        Relation::CrossRegion => "CROSS REGION",
        Relation::Unknown => "RELATION UNKNOWN",
    }
}

fn relation_message(relation: Relation) -> &'static str {
    match relation {
        Relation::Off => "REQUESTER OFF · BASELINE CELL",
        Relation::SameRegion => "SAME REGION · SHARED MEMORY TARGET",
        Relation::CrossRegion => "CROSS REGION · DISTINCT TARGETS / SHARED PATH POSSIBLE",
        Relation::Unknown => "RELATION UNKNOWN · METADATA INCOMPLETE",
    }
}

fn relation_color(relation: Relation) -> Color {
    match relation {
        Relation::Off => SUCCESS,
        Relation::SameRegion => DANGER,
        Relation::CrossRegion => ACCENT_ALT,
        Relation::Unknown => WARNING,
    }
}

fn activity_label(activity: Activity) -> &'static str {
    match activity {
        Activity::Inactive => "idle",
        Activity::Workload => "WORKLOAD",
        Activity::Requester => "REQUESTER",
        Activity::Collision => "WORKLOAD + REQUESTER",
    }
}

fn region_activity_label(region: &MemoryRegionState) -> String {
    match region
        .active_placement
        .as_ref()
        .filter(|arena| arena.id != region.placement.id)
    {
        Some(arena) => format!(
            "{} · {}",
            activity_label(region.activity),
            placement_label(arena)
        ),
        None => activity_label(region.activity).to_string(),
    }
}

fn activity_color(activity: Activity) -> Color {
    match activity {
        Activity::Inactive => DIM,
        Activity::Workload => ACCENT,
        Activity::Requester => WARNING,
        Activity::Collision => DANGER,
    }
}

fn footprint_label(state: &ExperimentState) -> String {
    state
        .working_set_bytes
        .map(format_footprint_size)
        .unwrap_or_else(|| "working set unavailable".to_string())
}

fn arena_range(placement: &Placement) -> String {
    placement
        .range
        .end()
        .map(|end| {
            format!(
                "[{}, {})",
                format_address(placement.range.start),
                format_address(end)
            )
        })
        .unwrap_or_else(|| "invalid address range".to_string())
}

fn format_address(address: u64) -> String {
    format!("0x{address:08x}")
}

fn format_arena_size(bytes: u64) -> String {
    format!("{} B", comma(bytes))
}

fn format_footprint_size(bytes: u64) -> String {
    if bytes.is_multiple_of(1024) {
        format!("{} KiB", comma(bytes / 1024))
    } else {
        format_arena_size(bytes)
    }
}

fn observed_clock(adapter: &LxV2Adapter) -> Option<u32> {
    adapter
        .metadata()
        .map(|metadata| metadata.cyccnt_hz)
        .filter(|clock| *clock != 0)
}

fn format_cycle_time(cycles: u64, clock_hz: Option<u32>) -> String {
    match clock_hz {
        Some(clock) => format!(
            "{} cyc · {:.3} ms",
            comma(cycles),
            cycles as f64 * 1000.0 / clock as f64
        ),
        None => format!("{} cyc", comma(cycles)),
    }
}

fn optional_cycles(cycles: Option<u64>) -> String {
    cycles.map(comma).unwrap_or_else(|| "waiting".to_string())
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

fn comparison_change(mode: Mode, stats: &PlacementStats, lowest: Option<u64>) -> String {
    let penalty = match mode {
        Mode::Baseline => stats
            .p50()
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
    let device = match options.profile.as_deref() {
        Some(path) => profile::load_profile(path)
            .map_err(|error| format!("cannot load device profile {}: {error}", path.display()))?,
        None => profile::stm32u585()
            .map_err(|error| format!("invalid built-in STM32U585 profile: {error}"))?,
    };
    if options.headless {
        return run_headless(&options, device);
    }
    ratatui::run(|terminal| run_tui(terminal, &options, device)).map_err(|error| error.to_string())
}

fn run_tui(terminal: &mut DefaultTerminal, options: &Options, device: Device) -> io::Result<()> {
    let mut app = App::with_device(options, device);
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
        app.prepare_semantic_effects();
        let elapsed = now.duration_since(last_draw);
        last_draw = now;
        terminal.draw(|frame| app.draw(frame, elapsed, now))?;

        if event::poll(Duration::from_millis(33))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.should_quit = true
                        }
                        KeyCode::Char('m') => {
                            app.view = match app.view {
                                View::Telemetry => View::Memory,
                                View::Memory => View::Telemetry,
                            }
                        }
                        KeyCode::Tab if app.view == View::Memory => {
                            app.metric = app.metric.next();
                        }
                        KeyCode::Up if app.view == View::Memory => {
                            let count = app.device.device.memory_regions.len();
                            if count > 0 {
                                app.selected_region = (app.selected_region + count - 1) % count;
                            }
                        }
                        KeyCode::Down if app.view == View::Memory => {
                            let count = app.device.device.memory_regions.len();
                            if count > 0 {
                                app.selected_region = (app.selected_region + 1) % count;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    app.finish_recording()
}

fn run_headless(options: &Options, device: Device) -> Result<(), String> {
    let live = !matches!(options.source, SourceSpec::File(_));
    if live && options.duration.is_none() {
        return Err("live --headless requires --duration".to_string());
    }

    let mut app = App::with_device(options, device);
    let started = Instant::now();
    if let Some(error) = app.source.error() {
        return Err(format!("source open failed: {error}"));
    }
    loop {
        app.poll_source();
        if let Some(error) = app.source.error() {
            return Err(format!("source read failed: {error}"));
        }
        if app.source_finished
            || options
                .duration
                .is_some_and(|duration| started.elapsed() >= duration)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    app.finish_recording().map_err(|error| error.to_string())?;

    print!("{}", headless_summary(&app.telemetry.stats));
    if app.telemetry.latest_header.is_none() {
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
    let mut profile = None;
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
            "--profile" => profile = Some(PathBuf::from(required_value(&mut args, "--profile")?)),
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
        profile,
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

fn fit_status_labels(
    source: &str,
    profile: Option<&str>,
    record: &str,
    width: usize,
) -> (String, Option<String>, String) {
    let Some(profile) = profile else {
        let (source, record) = fit_header_pair(source, record, width);
        return (source, None, record);
    };

    let fixed_width = Span::raw(PROFILE_SEPARATOR).width()
        + Span::raw(SIMULATED_SEPARATOR).width()
        + Span::raw(SIMULATED_LABEL).width()
        + Span::raw(SIMULATED_RECORD_SEPARATOR).width();
    let label_width = width.saturating_sub(fixed_width);
    let source_width = Span::raw(source).width();
    let profile_width = Span::raw(profile).width();
    let record_width = Span::raw(record).width();
    let mut remaining = label_width;
    let mut source_budget = source_width.min(3).min(remaining);
    remaining -= source_budget;
    let mut profile_budget = profile_width.min(3).min(remaining);
    remaining -= profile_budget;
    let mut record_budget = record_width.min(3).min(remaining);
    remaining -= record_budget;

    for (budget, wanted) in [
        (&mut profile_budget, profile_width),
        (&mut source_budget, source_width),
        (&mut record_budget, record_width),
    ] {
        let extra = wanted.saturating_sub(*budget).min(remaining);
        *budget += extra;
        remaining -= extra;
    }

    (
        elide_middle(source, source_budget),
        Some(elide_middle(profile, profile_budget)),
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

fn region_color(index: usize) -> Color {
    match index % 5 {
        0 => ACCENT,
        1 => ACCENT_ALT,
        2 => WARNING,
        3 => SUCCESS,
        4 => Color::Rgb(174, 154, 214),
        _ => FG,
    }
}

fn print_help() {
    println!(
        r#"laxity-tui

hardware-independent logical memory and exact-cell Laxity telemetry

usage:
  laxity-tui --udp PORT [--record PATH] [--duration SECONDS] [--profile PATH]
  laxity-tui --serial [PATH] [--record PATH] [--duration SECONDS] [--profile PATH]
  laxity-tui --file PATH [--headless] [--record PATH] [--profile PATH]

sources:
  --udp PORT       listen on 0.0.0.0:PORT
  --serial [PATH]  read 921600 8N1; auto-detect one ST-LINK when omitted
  --file PATH      replay a byte-for-byte capture at demo pace

device model:
  --profile PATH   load a capability and logical-memory profile; default is STM32U585

live control:
  --duration SEC   stop a UDP or serial session cleanly after SEC; default is no limit
  --headless       print counters; live sources also require --duration

recording:
  live sources write telemetry.bin in the launch directory and rotate if it exists
  file replay does not record unless --record is provided

interpretation:
  p50 retains the newest 4,096 samples per exact cell
  off p50 is this placement with aggressor_idx zero
  LX v2 contention cells match the exact requester target and footprint index
  cross region means distinct logical targets; the physical path may be shared
  observed best is measured, not a planner decision
  exec_cyc can contain both preemption and memory contention

serial ownership:
  run either this TUI or capture.sh on one serial port, never both

keys:
  m                switch telemetry and memory views
  tab              change memory metric
  up, down         select a logical memory region
  q, esc, ctrl-c   quit"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};
    use std::fs;
    use telemetry::{
        LxHeader, LxPlacement, LxRecord, Metadata, PLACEMENT_ALT_ADDR, PLACEMENT_CONTROL,
    };

    fn temp_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("laxity-tui-{label}-{}-{nonce}", std::process::id()))
    }

    fn test_record(seq: u32, placement_id: u8, aggressor_idx: u16, exec_cyc: u32) -> LxRecord {
        LxRecord {
            seq,
            release_cyc: 0,
            exec_cyc,
            cpu_cyc: 0,
            stall_cyc: 0,
            model_id: 0,
            placement_id,
            flags: 0,
            aggressor_idx,
            padding: 0,
            reserved: seq,
        }
    }

    fn test_options(source: SourceSpec, record: Option<PathBuf>) -> Options {
        Options {
            source,
            record,
            profile: None,
            headless: false,
            duration: None,
        }
    }

    fn test_app(options: &Options) -> App {
        App::with_device(options, profile::stm32u585().unwrap())
    }

    fn install_placements(app: &mut App, placements: Vec<LxPlacement>) {
        let changed = app.adapter.apply_header(
            &mut app.device,
            LxHeader {
                metadata: Metadata {
                    version: 2,
                    clock_hz: 160_000_000,
                    cyccnt_hz: 159_999_900,
                    n_placements: placements.len() as u8,
                    record_size: 32,
                    ..Metadata::default()
                },
                placements,
            },
        );
        assert!(changed);
        app.measurements
            .begin_revision(app.device.topology_revision);
    }

    fn install_origin(app: &mut App, simulated: bool) {
        app.adapter.apply_header(
            &mut app.device,
            LxHeader {
                metadata: Metadata {
                    version: 2,
                    simulated,
                    record_size: 32,
                    ..Metadata::default()
                },
                placements: Vec::new(),
            },
        );
    }

    fn raw_placement(id: u8, flags: u8, name: &str, address: u32) -> LxPlacement {
        LxPlacement {
            id,
            flags,
            name: name.to_string(),
            rel_cost: 1000,
            arena_addr: address,
            arena_size: 2944,
        }
    }

    fn observe_record(app: &mut App, record: LxRecord) -> model::Measurement {
        let measurement = app.adapter.adapt_record(&app.device, record);
        app.measurements.observe(measurement.clone());
        measurement
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
        let options = test_options(SourceSpec::File(PathBuf::from("/missing")), None);
        let mut app = test_app(&options);
        install_placements(
            &mut app,
            vec![
                raw_placement(1, 0, "SRAM1", 0x2000_2000),
                raw_placement(2, 0, "SRAM2", 0x2003_0000),
                raw_placement(3, 0, "SRAM3", 0x2004_0000),
                raw_placement(5, PLACEMENT_CONTROL, "SRAM1c", 0x2000_2000),
                raw_placement(6, PLACEMENT_ALT_ADDR, "SRAM2b", 0x2003_9000),
            ],
        );
        for (seq, region, cycles) in [(1, 1, 320_898), (2, 2, 320_901), (3, 3, 320_899)] {
            observe_record(&mut app, test_record(seq, region, 0, cycles));
        }
        let mut current = None;
        for (seq, region, cycles) in [(10, 1, 338_414), (11, 2, 321_414), (12, 3, 321_535)] {
            let measurement = observe_record(&mut app, test_record(seq, region, 0x0001, cycles));
            if seq == 10 {
                current = Some(measurement);
            }
        }
        app.display_state = Some(ExperimentState::from_model(
            &app.device,
            &app.measurements,
            current.unwrap(),
        ));
        app.display_revision = app.device.topology_revision;
        app
    }

    #[test]
    fn missing_source_renders_diagnostics_instead_of_a_blank_screen() {
        let options = test_options(
            SourceSpec::File(PathBuf::from(
                "/path/that/does/not/exist/laxity-telemetry.bin",
            )),
            None,
        );
        let mut app = test_app(&options);
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
        assert!(screen.contains("logical memory topology"));
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
        let options = test_options(SourceSpec::File(PathBuf::from("/missing")), None);
        let mut app = test_app(&options);
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
            let options = test_options(SourceSpec::File(path.clone()), None);
            let mut app = test_app(&options);
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
    fn simulated_header_shows_profile_and_unmissable_origin_badge() {
        let options = test_options(
            SourceSpec::File(PathBuf::from("/missing")),
            Some(PathBuf::from("telemetry-1789445000.bin")),
        );
        let mut app = App::with_device(&options, profile::virtual_generic().unwrap());
        app.source = Input::Udp {
            socket: UdpSocket::bind(("127.0.0.1", 0)).unwrap(),
            port: 50_505,
            peer: None,
        };
        install_origin(&mut app, true);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let now = Instant::now();
        let frame = terminal
            .draw(|frame| app.draw(frame, Duration::ZERO, now))
            .unwrap();
        let row: String = (0..80)
            .filter_map(|x| frame.buffer.cell((x, 2)))
            .map(|cell| cell.symbol())
            .collect();

        assert!(row.contains("udp 0.0.0.0:50505"));
        assert!(row.contains("profile virtual-generic"));
        assert!(row.contains("SIMULATED"));

        let cells: Vec<_> = (0..80).filter_map(|x| frame.buffer.cell((x, 2))).collect();
        let badge = cells
            .windows("SIMULATED".len())
            .find(|window| {
                window.iter().map(|cell| cell.symbol()).collect::<String>() == "SIMULATED"
            })
            .unwrap();
        assert!(badge.iter().all(|cell| cell.fg == Color::Black));
        assert!(badge.iter().all(|cell| cell.bg == DANGER));
        assert!(badge
            .iter()
            .all(|cell| cell.modifier.contains(Modifier::BOLD)));
    }

    #[test]
    fn real_header_has_no_origin_or_profile_field() {
        let options = test_options(SourceSpec::File(PathBuf::from("real.bin")), None);
        let mut app = test_app(&options);
        install_origin(&mut app, false);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let now = Instant::now();
        let frame = terminal
            .draw(|frame| app.draw(frame, Duration::ZERO, now))
            .unwrap();
        let row: String = (0..80)
            .filter_map(|x| frame.buffer.cell((x, 2)))
            .map(|cell| cell.symbol())
            .collect();

        assert!(!row.contains("SIMULATED"));
        assert!(!row.contains("profile"));
    }

    #[test]
    fn metric_colors_treat_negative_slack_as_a_miss() {
        assert_eq!(metric_color(ViewMetric::Slack, Some(-1.0)), DANGER);
        assert_eq!(metric_color(ViewMetric::Slack, Some(1.0)), SUCCESS);
        assert_eq!(metric_color(ViewMetric::Penalty, Some(-1.0)), SUCCESS);
        assert_eq!(metric_color(ViewMetric::Penalty, Some(1.0)), WARNING);
        assert_eq!(metric_color(ViewMetric::P50, None), FAINT);
    }

    #[test]
    fn pending_record_becomes_visible_after_the_display_hold() {
        let mut app = populated_app();
        let before = app.display_state.as_ref().unwrap().measurement.sequence;
        let now = Instant::now();
        app.last_display_update = Some(now);
        observe_record(&mut app, test_record(99, 2, 0x0103, 330_000));

        app.update_display_record(now + DISPLAY_HOLD - Duration::from_millis(1), false);
        assert_eq!(
            app.display_state.as_ref().unwrap().measurement.sequence,
            before
        );

        app.update_display_record(now + DISPLAY_HOLD, false);
        assert_eq!(app.display_state.as_ref().unwrap().measurement.sequence, 99);
    }

    #[test]
    fn placement_metadata_change_starts_a_fresh_measurement_revision() {
        let mut app = populated_app();
        let old_revision = app.device.topology_revision;
        let changed = LxHeader {
            metadata: Metadata {
                version: 2,
                n_placements: 1,
                record_size: 32,
                ..Metadata::default()
            },
            placements: vec![raw_placement(2, 0, "moved", 0x2003_1000)],
        };

        app.accept_delta(
            ParseDelta {
                events: vec![LxEvent::Header(changed)],
                valid_frames: 1,
                ..ParseDelta::default()
            },
            Instant::now(),
        );

        assert!(app.device.topology_revision > old_revision);
        assert!(app.measurements.latest().is_none());
        assert!(app.display_state.is_none());
    }

    #[test]
    fn crc_rejection_requests_a_local_transport_warning() {
        let options = test_options(SourceSpec::File(PathBuf::from("/missing")), None);
        let mut app = test_app(&options);

        app.accept_delta(
            ParseDelta {
                rejected_candidates: 1,
                ..ParseDelta::default()
            },
            Instant::now(),
        );

        assert_eq!(
            app.semantic_effects.last(),
            Some(&SemanticEffect {
                target: EffectTarget::TransportHeader,
                kind: EffectKind::Warning,
            })
        );
    }

    #[test]
    fn zero_cycle_median_renders_without_dividing_by_zero() {
        let options = test_options(SourceSpec::File(PathBuf::from("/missing")), None);
        let mut app = test_app(&options);
        install_placements(&mut app, vec![raw_placement(3, 0, "SRAM3", 0x2004_0000)]);
        observe_record(&mut app, test_record(1, 3, 0, 0));
        let current = observe_record(&mut app, test_record(2, 3, 3, 1));
        app.display_state = Some(ExperimentState::from_model(
            &app.device,
            &app.measurements,
            current,
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
        assert!(screen.contains("SAME REGION"));
        assert!(screen.contains("0x20002000"));
        assert!(screen.contains("observed best"));
        assert!(!screen.contains("SRAM1c"));
    }

    #[test]
    fn memory_view_renders_address_slabs_and_requester_flow_at_eighty_by_twenty_four() {
        let mut app = populated_app();
        app.view = View::Memory;
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

        for text in ["logical address slabs", "SRAM1", "0x20000000", "GPDMA1"] {
            assert!(screen.contains(text), "missing {text}");
        }
        assert!(screen.contains("physical interconnect unavailable"));
    }

    #[test]
    fn memory_view_accepts_an_arbitrary_non_stm32_profile() {
        let options = test_options(SourceSpec::File(PathBuf::from("/missing")), None);
        let mut app = App::with_device(&options, profile::virtual_generic().unwrap());
        app.view = View::Memory;
        let mut terminal = Terminal::new(TestBackend::new(100, 28)).unwrap();
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

        for text in ["Virtual generic target", "MEM-A", "MEM-B", "MEM-C"] {
            assert!(screen.contains(text), "missing {text}");
        }
        assert!(screen.contains("p50 unavailable"));
        assert!(!screen.contains("SRAM"));
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
            "logical memory topology",
            "current observed cell",
            "placement comparison",
            "SAME REGION",
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
            "requester proof",
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
        let options = test_options(SourceSpec::File(path.clone()), None);
        let mut app = test_app(&options);
        let now = Instant::now();
        app.last_byte = Some(now);
        app.last_frame = Some(now);

        assert_eq!(app.health(now), Health::ReplayRunning);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn recorder_writes_received_bytes_exactly_once() {
        let path = temp_path("record-exact");
        let options = test_options(
            SourceSpec::File(PathBuf::from("/missing")),
            Some(path.clone()),
        );
        let bytes = b"boot\r\nLX\x02\x01\x00\x00\xff\xff\x00";
        let mut app = test_app(&options);

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
        let options = test_options(
            SourceSpec::File(PathBuf::from("/missing")),
            Some(path.clone()),
        );
        let mut app = test_app(&options);

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
    fn known_real_capture_keeps_golden_accounting_and_experiment_state() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../results/raw/20260909T175908Z-contention-sweep/telemetry.bin");
        if !path.exists() {
            writeln!(
                io::stderr().lock(),
                "skipping real capture regression; expected {}",
                path.display()
            )
            .unwrap();
            return;
        }
        let bytes = fs::read(&path).unwrap();
        let options = test_options(SourceSpec::File(path), None);
        let mut app = test_app(&options);
        for chunk in bytes.chunks(997) {
            app.ingest(chunk);
        }
        let delta = app.telemetry.finish();
        app.accept_delta(delta, Instant::now());

        assert_eq!(bytes.len(), 1_769_499);
        assert_eq!(app.telemetry.stats.frames, 18_124);
        assert_eq!(app.telemetry.stats.accepted_frames, 18_124);
        assert_eq!(app.telemetry.stats.header_frames, 9_062);
        assert_eq!(app.telemetry.stats.batch_frames, 9_062);
        assert_eq!(app.telemetry.stats.records, 9_062);
        assert_eq!(app.telemetry.stats.skipped_bytes, 65_843);
        assert_eq!(app.telemetry.stats.wrapped, 1);
        assert_eq!(app.telemetry.stats.gaps, 0);
        assert_eq!(app.telemetry.stats.dropped, 0);
        assert_eq!(app.telemetry.stats.false_sync, 0);
        assert_eq!(app.telemetry.stats.crc_rejections, 0);
        assert!(!app.adapter.metadata().unwrap().simulated);

        let latest = app.measurements.latest().cloned().unwrap();
        let experiment = ExperimentState::from_model(&app.device, &app.measurements, latest);
        assert_eq!(experiment.mode, Mode::Contention);
        assert_eq!(experiment.relation, Relation::CrossRegion);
        assert_eq!(
            experiment.inference.as_ref().unwrap().label.as_deref(),
            Some("SRAM2")
        );
        assert_eq!(
            experiment
                .requester_target
                .as_ref()
                .unwrap()
                .label
                .as_deref(),
            Some("SRAM1")
        );
        assert_eq!(experiment.working_set_bytes, Some(1024));
        assert_eq!(experiment.current.unwrap().p50, 321_414);
        assert_eq!(experiment.baseline.unwrap().p50, 320_901);
        assert_eq!(experiment.penalty.unwrap().cycles, 513);
        assert_eq!(experiment.observed_spread, Some(17_000));
        assert_eq!(
            experiment
                .comparisons
                .iter()
                .find(|stats| stats.observed_best)
                .unwrap()
                .placement
                .label
                .as_deref(),
            Some("SRAM2")
        );
    }

    #[test]
    fn duration_requires_positive_whole_seconds() {
        assert_eq!(parse_duration("10").unwrap(), Duration::from_secs(10));
        assert!(parse_duration("0").is_err());
        assert!(parse_duration("1.5").is_err());
    }

    #[test]
    fn live_headless_mode_requires_a_duration() {
        let options = Options {
            source: SourceSpec::Udp(50505),
            record: None,
            profile: None,
            headless: true,
            duration: None,
        };

        assert_eq!(
            run_headless(&options, profile::stm32u585().unwrap()).unwrap_err(),
            "live --headless requires --duration"
        );
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
