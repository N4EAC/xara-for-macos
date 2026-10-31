//! In-process macOS modem lifecycle and its narrow C ABI. No child processes.
use crate::audio::ratematch::TxBuffer;
use crate::audio::txpath::TxPath;
use crate::audio::{self, Audio};
use crate::ptt::{PttCtl, PttSpec, PttTiming};
use crate::station::{self, Modem, Options};
use crate::tnc::Tnc;
use std::ffi::{CStr, c_char};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use xara_arq::{fm, hf};
use xara_host::kiss::KissServer;
use xara_host::server::Host;
use xara_phy::fm::level::FmBandwidth;
use xara_phy::hf::level::Bandwidth;
use xara_ui::json::Obj;
use xara_ui::messages::Info;
use xara_ui::server::{Access, UiServer, new_token};
use xara_ui::settings::{self, Device, Devices, Init, Pairs, Settings};
use xara_ui::telemetry::{self, EVENT_QUEUE};

#[derive(Clone, Debug)]
struct Config {
    call: String,
    hf: Bandwidth,
    fm: Option<FmBandwidth>,
    capture: String,
    playback: String,
    drive: f64,
    timing: PttTiming,
    ptt: Option<PttSpec>,
    port: u16,
    host_ports: bool,
    kiss_port: Option<u16>,
}

impl Config {
    fn parse(text: &str) -> Result<Self, String> {
        let mut config = Self {
            call: String::new(),
            hf: Bandwidth::Hz2300,
            fm: None,
            capture: "default".into(),
            playback: "default".into(),
            drive: -5.0,
            timing: PttTiming::default(),
            ptt: None,
            port: 8300,
            host_ports: false,
            kiss_port: Some(8100),
        };
        for entry in settings::entries(text)? {
            config.set(&entry.key, entry.value.as_deref().unwrap_or_default())?;
        }
        config.validate()?;
        Ok(config)
    }

    fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let number = |v: &str| v.parse::<u64>().map_err(|_| format!("invalid {key}"));
        match key {
            "call" => self.call = crate::cli::call(value)?,
            "hf" => {
                self.hf = value.parse()?;
                self.fm = None;
            }
            "fm" => {
                self.fm = Some(match value {
                    "narrow" => FmBandwidth::Narrow,
                    "wide" => FmBandwidth::Wide,
                    _ => return Err("invalid FM mode".into()),
                })
            }
            "capture" => self.capture = value.into(),
            "playback" => self.playback = value.into(),
            "drive-db" => self.drive = value.parse().map_err(|_| "invalid TX drive")?,
            "ptt-lead-ms" => self.timing.lead = Duration::from_millis(number(value)?),
            "ptt-tail-ms" => self.timing.tail = Duration::from_millis(number(value)?),
            "ptt-max-s" => self.timing.max_on = Duration::from_secs(number(value)?),
            "ptt" => self.ptt = if value.is_empty() || value == "off" { None } else { Some(PttSpec::parse(value)?) },
            "port" => self.port = value.parse().map_err(|_| "invalid command port")?,
            "host-ports" => {
                self.host_ports = match value {
                    "on" => true,
                    "off" => false,
                    _ => return Err("host-ports must be on or off".into()),
                }
            }
            "kiss-port" => {
                self.kiss_port =
                    if value == "off" { None } else { Some(value.parse().map_err(|_| "invalid KISS port")?) }
            }
            _ => return Err(format!("unsupported app setting {key}")),
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), String> {
        if self.ptt.is_some() && !self.capture.starts_with("coreaudio:") {
            return Err("Select an explicit radio input device before enabling transmit.".into());
        }
        if self.call.is_empty() || self.call.split('-').next().is_some_and(|s| s.eq_ignore_ascii_case("N0CALL")) {
            return Err("Enter your station callsign before starting the modem.".into());
        }
        if self.capture.is_empty() || self.playback.is_empty() {
            return Err("Select audio input and output.".into());
        }
        if !self.drive.is_finite() || !(-30.0..=10.0).contains(&self.drive) {
            return Err("TX drive must be -30 to +10 dB.".into());
        }
        if self.timing.lead > Duration::from_secs(1)
            || self.timing.tail > Duration::from_secs(1)
            || self.timing.max_on.is_zero()
            || self.timing.max_on > Duration::from_secs(120)
        {
            return Err("Invalid PTT timing.".into());
        }
        if self.port == 0
            || self.port == 65535
            || self.kiss_port == Some(0)
            || self.kiss_port.is_some_and(|p| p == self.port || p == self.port + 1)
        {
            return Err("Command/data and KISS ports must be distinct, nonzero ports.".into());
        }
        if matches!(self.ptt, Some(PttSpec::Cm108 { .. } | PttSpec::Trusdx)) {
            return Err("Use serial RTS/DTR, CM108 USB, rigctld, or VOX in this app.".into());
        }
        if self.ptt.is_some() {
            audio::coreaudio::check(&decode_device(&self.playback)?, true).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn with_saved(&self, saved: &Pairs) -> Result<Self, String> {
        let mut next = self.clone();
        for (key, value) in saved {
            next.set(key, value)?;
        }
        next.validate()?;
        Ok(next)
    }

    fn host(&self) -> Result<Host, String> {
        if self.host_ports {
            Host::bind("127.0.0.1", self.port).map_err(|e| format!("Command/data ports: {e}"))
        } else {
            Ok(Host::disabled())
        }
    }

    fn values(&self) -> Pairs {
        let mode =
            self.fm.map(|bw| ("fm", format!("{bw:?}").to_lowercase())).unwrap_or(("hf", self.hf.hz().to_string()));
        [
            ("call", self.call.clone()),
            mode,
            ("capture", self.capture.clone()),
            ("playback", self.playback.clone()),
            ("drive-db", self.drive.to_string()),
            ("ptt-lead-ms", self.timing.lead.as_millis().to_string()),
            ("ptt-tail-ms", self.timing.tail.as_millis().to_string()),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v))
        .collect()
    }
}

#[derive(Default)]
struct Status {
    state: i32,
    text: String,
}
struct Runtime {
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<Status>>,
    worker: JoinHandle<()>,
}
static CAN_CONFIGURE: AtomicBool = AtomicBool::new(true);
static RUNTIME: Mutex<Option<Runtime>> = Mutex::new(None);

fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|e| e.into_inner())
}
fn update(status: &Mutex<Status>, state: i32, text: String) {
    *lock(status) = Status { state, text };
}

fn encode_device(uid: &str) -> String {
    format!("coreaudio:{}", uid.as_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>())
}

fn decode_device(value: &str) -> Result<String, String> {
    if value == "default" {
        return Ok(value.into());
    }
    let hex = value.strip_prefix("coreaudio:").ok_or("Select a Core Audio device.")?;
    if hex.is_empty() || hex.len() % 2 != 0 || hex.len() > 2048 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid Core Audio device identifier.".into());
    }
    let bytes: Result<Vec<_>, _> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16)).collect();
    String::from_utf8(bytes.map_err(|_| "Invalid device identifier.")?).map_err(|_| "Invalid device identifier.".into())
}

fn devices() -> std::io::Result<Devices> {
    let (capture, playback) = audio::coreaudio::list()?;
    let map = |v: Vec<(String, String)>| {
        v.into_iter().map(|(name, desc)| Device { name: encode_device(&name), desc }).collect()
    };
    Ok(Devices { backend: "coreaudio", capture: map(capture), playback: map(playback) })
}

fn private_state_dir(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    if std::fs::symlink_metadata(dir).map_err(|e| e.to_string())?.file_type().is_symlink() {
        return Err("The app state directory must not be a symbolic link.".into());
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    let path = dir.join(settings::FILE);
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err("The settings path must be a regular file.".into());
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn run(base: Config, dir: PathBuf, stop: Arc<AtomicBool>, status: Arc<Mutex<Status>>) -> Result<(), String> {
    private_state_dir(&dir)?;
    let token = new_token().map_err(|e| e.to_string())?;
    let mut ui_port = 0;
    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        let saved = settings::load(&dir)?;
        let cfg = base.with_saved(&saved)?;
        let check = base.clone();
        let settings = Arc::new(Settings::new(Init {
            dir: Some(dir.clone()),
            values: cfg.values(),
            saved,
            error: None,
            validate: Box::new(move |saved| check.with_saved(saved).map(|_| ())),
            list: Box::new(devices),
        }));
        let capture = decode_device(&cfg.capture)?;
        let mut io = if cfg.ptt.is_none() {
            audio::coreaudio::CoreAudioIo::open_capture(&capture)
        } else {
            audio::coreaudio::CoreAudioIo::open(&capture, &decode_device(&cfg.playback)?, true, TxBuffer::default())
        }
        .map_err(|e| e.to_string())?;
        let tx_path =
            TxPath::new(audio::samples(cfg.timing.lead), TxBuffer::default(), io.path().map_err(|e| e.to_string())?);
        let budget = tx_path.budget();
        let host = cfg.host()?;
        let opts = Options {
            drive_db: cfg.drive,
            tx_latency: budget.total(),
            rx_lag: budget.lag(),
            ptt_tail: audio::samples(cfg.timing.tail) as i64,
        };
        let result = if let Some(bw) = cfg.fm {
            let mut tnc = Tnc::new(station::fm::Station::new(fm::Config::new(bw, &cfg.call), opts), host);
            serve(
                &cfg,
                &mut io,
                &mut tnc,
                settings,
                Workspace { stop: &stop, status: &status, token: &token, port: &mut ui_port, tx_path: Some(tx_path) },
            )
        } else {
            let mut tnc = Tnc::new(station::hf::Station::new(hf::Config::new(cfg.hf, &cfg.call), opts), host);
            serve(
                &cfg,
                &mut io,
                &mut tnc,
                settings,
                Workspace { stop: &stop, status: &status, token: &token, port: &mut ui_port, tx_path: Some(tx_path) },
            )
        }?;
        if !result || stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        update(&status, 1, "Applying modem settings…".into());
    }
}

#[derive(Default)]
struct DriveState {
    pending: Option<f64>,
    stopping: bool,
    result: Option<Result<(), String>>,
}

/// Coalesces drive updates off the DSP thread and flushes the last accepted value on shutdown.
struct DriveSaver {
    state: Arc<(Mutex<DriveState>, Condvar)>,
    worker: Option<JoinHandle<()>>,
}

impl DriveSaver {
    fn new(settings: Arc<Settings>) -> Result<Self, String> {
        let state = Arc::new((Mutex::new(DriveState::default()), Condvar::new()));
        let shared = state.clone();
        let worker = thread::Builder::new()
            .name("xara-drive-settings".into())
            .spawn(move || {
                let (mutex, changed) = &*shared;
                loop {
                    let state = lock(mutex);
                    let mut state = changed
                        .wait_while(state, |s| s.pending.is_none() && !s.stopping)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let Some(db) = state.pending.take() else { return };
                    drop(state);
                    let result = settings.save(&vec![("drive-db".into(), db.to_string())]).map(|_| ());
                    lock(mutex).result = Some(result);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { state, worker: Some(worker) })
    }

    fn queue(&self, db: f64) {
        lock(&self.state.0).pending = Some(db);
        self.state.1.notify_one();
    }

    fn result(&self) -> Option<Result<(), String>> {
        lock(&self.state.0).result.take()
    }

    fn finish(&mut self) -> Result<(), String> {
        lock(&self.state.0).stopping = true;
        self.state.1.notify_one();
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| "Drive settings worker failed.")?;
        }
        self.result().unwrap_or(Ok(()))
    }
}

impl Drop for DriveSaver {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

struct Workspace<'a> {
    stop: &'a Arc<AtomicBool>,
    status: &'a Mutex<Status>,
    token: &'a str,
    port: &'a mut u16,
    tx_path: Option<TxPath>,
}

fn serve<M: Modem>(
    cfg: &Config,
    io: &mut dyn Audio,
    tnc: &mut Tnc<M>,
    settings: Arc<Settings>,
    workspace: Workspace<'_>,
) -> Result<bool, String> {
    let Workspace { stop, status, token, port: ui_port, mut tx_path } = workspace;
    let mut ui = None;
    let mut drive_saver = None;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(port) = cfg.kiss_port.filter(|_| cfg.host_ports) {
            tnc.set_kiss(KissServer::bind("127.0.0.1", port).map_err(|e| format!("KISS port: {e}"))?);
        }
        let (telemetry, feed) = telemetry::channel(EVENT_QUEUE);
        let fm = cfg.fm.is_some();
        let info = Info {
            version: env!("CARGO_PKG_VERSION"),
            mode: if fm { "fm" } else { "hf" },
            span_hz: if fm { crate::FM_SPAN_HZ } else { crate::HF_SPAN_HZ },
        };
        let server = UiServer::bind(
            "127.0.0.1",
            *ui_port,
            feed,
            info,
            Access {
                control: true,
                private: true,
                token: Some(token.into()),
                settings: Some(settings.clone()),
                hosts: Vec::new(),
            },
        )
        .map_err(|e| e.to_string())?;
        *ui_port = server.addr.port();
        let url = format!("http://127.0.0.1:{}/?token={token}", *ui_port);
        ui = Some(server);
        tnc.set_telemetry(telemetry);
        tnc.use_output_meter();
        drive_saver = Some(DriveSaver::new(settings)?);
        let saver = drive_saver.as_ref().unwrap();
        let mut last_drive = cfg.drive;
        tnc.set_echo(false);
        let mut ptt = cfg
            .ptt
            .as_ref()
            .map(|spec| spec.open().map(|dev| PttCtl::spawn(dev, cfg.timing, stop.clone())))
            .transpose()
            .map_err(|e| e.to_string())?;
        update(status, 2, url);
        let mut input = [0i16; audio::BLOCK];
        let mut rx = [0.0; audio::BLOCK];
        let mut tx = [0.0; audio::BLOCK];
        let mut tx_path = tx_path.take().unwrap_or_else(|| {
            TxPath::new(audio::samples(cfg.timing.lead), TxBuffer::default(), audio::Path::default())
        });
        if cfg.ptt.is_some() {
            tx_path.start(io).map_err(|e| e.to_string())?;
        }
        let mut budget: Option<crate::audio::txpath::Budget> = None;
        let pumped = (|| {
            while !stop.load(Ordering::SeqCst) {
                let reading = Instant::now();
                io.read(&mut input).map_err(|e| e.to_string())?;
                let waited = reading.elapsed();
                let backlog = io.backlog().map_err(|e| e.to_string())?;
                if let Some(b) = budget.take() {
                    tnc.set_tx_latency(b.total(), b.lag());
                }
                for (x, sample) in rx.iter_mut().zip(input) {
                    *x = audio::sample(sample);
                }
                CAN_CONFIGURE.store(tnc.station().idle() && !tnc.keyed() && !tnc.station().tuning(), Ordering::SeqCst);
                let forced = cfg.ptt.is_none() || ptt.as_ref().is_some_and(PttCtl::forced_off);
                tnc.force_ptt_off(forced);
                tnc.process(&rx, &mut tx);
                let drive = tnc.station().options().drive_db;
                if drive != last_drive {
                    saver.queue(drive);
                    last_drive = drive;
                }
                if let Some(result) = saver.result() {
                    tnc.drive_saved(result.as_ref().err().map(String::as_str));
                }
                let keyed = tnc.keyed();
                if keyed {
                    if let Some(ptt) = &mut ptt {
                        ptt.set(true).map_err(|e| e.to_string())?;
                    }
                }
                if cfg.ptt.is_some() {
                    budget = tx_path.send(io, &mut tx, forced, waited, backlog).map_err(|e| e.to_string())?;
                } else {
                    tx.fill(0.0);
                }
                if !keyed {
                    if let Some(ptt) = &mut ptt {
                        ptt.set(false).map_err(|e| e.to_string())?;
                    }
                }
                let asserted = ptt.as_ref().is_some_and(PttCtl::keyed);
                tnc.publish_output(if cfg.ptt.is_some() { tx_path.sent() } else { &input[..0] }, asserted);
                if tnc.restart_requested() {
                    return Ok(true);
                }
            }
            Ok(false)
        })();
        if let Some(mut ptt) = ptt {
            ptt.finish().map_err(|e| e.to_string())?;
        }
        pumped
    }));
    let saved = drive_saver.as_mut().map_or(Ok(()), DriveSaver::finish);
    CAN_CONFIGURE.store(true, Ordering::SeqCst);
    tnc.shutdown();
    if let Some(ui) = ui {
        ui.shutdown();
    }
    match result {
        Ok(result) => result.and_then(|restart| saved.map(|_| restart)),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn write_text(text: &str, output: *mut c_char, capacity: usize) -> usize {
    let n = text.len();
    if !output.is_null() && capacity > 0 {
        let count = n.min(capacity - 1);
        // SAFETY: C callers provide an output allocation of at least capacity bytes; no overlap.
        unsafe {
            std::ptr::copy_nonoverlapping(text.as_ptr(), output.cast(), count);
            *output.add(count) = 0;
        }
    }
    n + 1
}

/// Start the engine worker. Returns 0 when scheduled, -1 for invalid input/already running.
/// # Safety
/// config and directory must point to valid null-terminated UTF-8 strings for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xara_embedded_start(config: *const c_char, directory: *const c_char) -> i32 {
    if config.is_null() || directory.is_null() {
        return -1;
    }
    // SAFETY: callers supply valid strings as documented above.
    let (config, directory) = unsafe { (CStr::from_ptr(config), CStr::from_ptr(directory)) };
    let mut runtime = lock(&RUNTIME);
    if runtime.is_some() {
        return -1;
    }
    let stop = Arc::<AtomicBool>::default();
    let status = Arc::new(Mutex::new(Status { state: 1, text: "Starting built-in modem…".into() }));
    let text = config.to_string_lossy().into_owned();
    let dir = Path::new(directory.to_str().unwrap_or_default()).to_path_buf();
    let (stopping, state) = (stop.clone(), status.clone());
    let worker = match thread::Builder::new().name("xara-modem".into()).spawn(move || {
        let result =
            std::panic::catch_unwind(|| Config::parse(&text).and_then(|cfg| run(cfg, dir, stopping, state.clone())));
        CAN_CONFIGURE.store(true, Ordering::SeqCst);
        match result {
            Ok(Ok(())) => update(&state, 0, "Modem stopped".into()),
            Ok(Err(error)) => update(&state, 3, error),
            Err(_) => update(&state, 3, "The modem stopped after an internal error.".into()),
        }
    }) {
        Ok(worker) => worker,
        Err(_) => return -1,
    };
    *runtime = Some(Runtime { stop, status, worker });
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn xara_embedded_stop() {
    if let Some(runtime) = lock(&RUNTIME).as_ref() {
        runtime.stop.store(true, Ordering::SeqCst);
    }
}

/// Join only after requesting stop; call off the main thread.
#[unsafe(no_mangle)]
pub extern "C" fn xara_embedded_wait() {
    if let Some(runtime) = lock(&RUNTIME).take() {
        let _ = runtime.worker.join();
    }
}

/// # Safety
/// output must be null or point to capacity writable bytes. Returns lifecycle state 0..3.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xara_embedded_status(output: *mut c_char, capacity: usize) -> i32 {
    if let Some(runtime) = lock(&RUNTIME).as_ref() {
        let status = lock(&runtime.status);
        write_text(&status.text, output, capacity);
        status.state
    } else {
        write_text("Modem stopped", output, capacity);
        0
    }
}

/// # Safety
/// output must be null or point to capacity writable bytes. Returns required capacity including NUL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xara_embedded_devices(output: *mut c_char, capacity: usize) -> usize {
    let text = match devices() {
        Ok(devices) => Obj::new()
            .arr("capture", devices.capture.iter().map(|d| Obj::new().str("name", &d.name).str("desc", &d.desc)))
            .arr("playback", devices.playback.iter().map(|d| Obj::new().str("name", &d.name).str("desc", &d.desc)))
            .end(),
        Err(error) => Obj::new().str("error", &error.to_string()).end(),
    };
    write_text(&text, output, capacity)
}

#[unsafe(no_mangle)]
pub extern "C" fn xara_embedded_can_configure() -> i32 {
    i32::from(CAN_CONFIGURE.load(Ordering::SeqCst))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    struct SimulatedInput(Arc<AtomicUsize>);
    impl Audio for SimulatedInput {
        fn read(&mut self, samples: &mut [i16]) -> std::io::Result<()> {
            samples.fill(0);
            self.0.fetch_add(1, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(1));
            Ok(())
        }
        fn write(&mut self, _: &[i16]) -> std::io::Result<()> {
            panic!("receive-only mode must never open or write audio output");
        }
        fn queued(&mut self) -> std::io::Result<Option<audio::ratematch::Queue>> {
            Ok(None)
        }
        fn backlog(&mut self) -> std::io::Result<Option<usize>> {
            Ok(None)
        }
        fn path(&mut self) -> std::io::Result<audio::Path> {
            Ok(audio::Path::default())
        }
    }

    fn session(
        cfg: Config,
        dir: PathBuf,
        port: u16,
        stop: Arc<AtomicBool>,
        status: Arc<Mutex<Status>>,
        reads: Arc<AtomicUsize>,
    ) -> JoinHandle<(bool, u16)> {
        thread::spawn(move || {
            let settings = Arc::new(Settings::new(Init {
                dir: Some(dir),
                values: cfg.values(),
                saved: Vec::new(),
                error: None,
                validate: Box::new(|_| Ok(())),
                list: Box::new(devices),
            }));
            let host = cfg.host().unwrap();
            let mut tnc =
                Tnc::new(station::hf::Station::new(hf::Config::new(cfg.hf, &cfg.call), Options::default()), host);
            let mut input = SimulatedInput(reads);
            let mut port = port;
            let restart = serve(
                &cfg,
                &mut input,
                &mut tnc,
                settings,
                Workspace {
                    stop: &stop,
                    status: &status,
                    token: "embedded-test-token",
                    port: &mut port,
                    tx_path: None,
                },
            )
            .unwrap();
            (restart, port)
        })
    }

    fn ready(status: &Mutex<Status>) -> SocketAddr {
        let started = Instant::now();
        loop {
            let current = lock(status);
            if current.state == 2 {
                return current.text.strip_prefix("http://").unwrap().split('/').next().unwrap().parse().unwrap();
            }
            drop(current);
            assert!(started.elapsed() < Duration::from_secs(5));
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn command(socket: &mut TcpStream, text: &str) {
        let mask = [1u8, 2, 3, 4];
        assert!(text.len() < 126);
        let mut frame = vec![0x81, 0x80 | text.len() as u8];
        frame.extend(mask);
        frame.extend(text.bytes().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        socket.write_all(&frame).unwrap();
    }

    #[test]
    fn embedded_workspace_saves_restarts_rebinds_and_stops_without_output() {
        let directory = std::env::temp_dir().join(format!("xara-embedded-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut cfg = Config::parse("call=W1AW\nkiss-port=off").unwrap();
        let stop = Arc::<AtomicBool>::default();
        let status = Arc::new(Mutex::new(Status::default()));
        let reads = Arc::<AtomicUsize>::default();
        let worker = session(cfg.clone(), directory.clone(), 0, stop.clone(), status.clone(), reads.clone());
        let address = ready(&status);
        let mut socket = TcpStream::connect(address).unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(socket, "GET /ws HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nCookie: xara_token_{}=embedded-test-token\r\n\r\n", address.port()).unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            socket.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        assert!(String::from_utf8(head).unwrap().starts_with("HTTP/1.1 101"));
        command(&mut socket, "DRIVE 10");
        command(&mut socket, "SETTINGS SAVE\ncall W2AW");
        command(&mut socket, "RESTART");
        let until = Instant::now() + Duration::from_secs(5);
        while !worker.is_finished() && Instant::now() < until {
            thread::sleep(Duration::from_millis(10));
        }
        stop.store(true, Ordering::SeqCst);
        let (restart, port) = worker.join().unwrap();
        assert!(restart, "the workspace must restart the in-process modem");
        assert!(reads.load(Ordering::SeqCst) > 10);
        let saved = settings::load(&directory).unwrap();
        assert!(saved.contains(&("call".into(), "W2AW".into())));
        cfg = cfg.with_saved(&saved).unwrap();
        assert_eq!(cfg.call, "W2AW");
        assert_eq!(cfg.drive, 10.0, "last accepted drive survives the full engine restart");
        stop.store(false, Ordering::SeqCst);
        update(&status, 1, String::new());
        let worker = session(cfg, directory.clone(), port, stop.clone(), status.clone(), reads);
        assert_eq!(ready(&status), address);
        stop.store(true, Ordering::SeqCst);
        assert!(!worker.join().unwrap().0);
        let _listener = std::net::TcpListener::bind(address).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn drive_saver_flushes_the_last_value_and_preserves_other_settings() {
        let dir = std::env::temp_dir().join(format!("xara-drive-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = Arc::new(Settings::new(Init {
            dir: Some(dir.clone()),
            values: Vec::new(),
            saved: vec![("call".into(), "W1AW".into())],
            error: None,
            validate: Box::new(|_| Ok(())),
            list: Box::new(|| Ok(Devices::default())),
        }));
        let mut saver = DriveSaver::new(settings).unwrap();
        for db in -30..=10 {
            saver.queue(f64::from(db));
        }
        saver.finish().unwrap();
        let saved = settings::load(&dir).unwrap();
        assert!(saved.contains(&("drive-db".into(), "10".into())));
        assert!(saved.contains(&("call".into(), "W1AW".into())));
        for value in ["-30", "0", "10"] {
            assert_eq!(
                Config::parse(&format!("call=W1AW\ndrive-db={value}")).unwrap().drive,
                value.parse::<f64>().unwrap()
            );
        }
        for value in ["10.1", "-30.1", "NaN", "inf"] {
            assert!(Config::parse(&format!("call=W1AW\ndrive-db={value}")).is_err());
        }
        assert_eq!(decode_device(&encode_device("USB Audio:UID-1")).unwrap(), "USB Audio:UID-1");
        assert!(decode_device("coreaudio:zz").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn private_state_repairs_permissions_and_refuses_symbolic_links() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = std::env::temp_dir().join(format!("xara-private-state-{}", std::process::id()));
        let alias = directory.with_extension("link");
        std::fs::create_dir_all(&directory).unwrap();
        let settings = directory.join(settings::FILE);
        let victim = directory.join("victim");
        std::fs::write(&settings, "call = W1AW\n").unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&settings, std::fs::Permissions::from_mode(0o644)).unwrap();
        private_state_dir(&directory).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&directory), 0o700);
        assert_eq!(mode(&settings), 0o600);
        std::fs::write(&victim, "keep").unwrap();
        std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::remove_file(&settings).unwrap();
        symlink(&victim, &settings).unwrap();
        assert!(private_state_dir(&directory).is_err());
        assert_eq!(mode(&victim), 0o644);
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        symlink(&directory, &alias).unwrap();
        assert!(private_state_dir(&alias).is_err());
        std::fs::remove_file(alias).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn config_rejects_invalid_identity_ports_and_radio_defaults() {
        let cfg = Config::parse("call=W1AW").unwrap();
        assert!(!cfg.host_ports, "external radio software must be explicitly enabled");
        assert_eq!(cfg.host().unwrap().cmd_addr.port(), 0);
        assert!(Config::parse("call=W1AW\nhost-ports=on").unwrap().host_ports);
        assert!(Config::parse("call=W1AW\nhost-ports=yes").is_err());
        assert!(Config::parse("call=N0CALL").is_err());
        assert!(Config::parse("call=W1AW\nport=65535").is_err());
        assert!(Config::parse("call=W1AW\nport=8100").is_err());
        assert!(Config::parse("call=W1AW\nptt=vox").is_err());
        assert!(Config::parse("call=W1AW\ndrive-db=NaN").is_err());
        assert!(Config::parse("call=W1AW\nptt=cm108:/dev/fake").is_err());
    }
}
