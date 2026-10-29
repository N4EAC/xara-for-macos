//! In-process macOS modem lifecycle and its narrow C ABI. No child processes.
use crate::audio::{self, Audio};
use crate::ptt::{PttCtl, PttSpec, PttTiming};
use crate::station::{self, Modem, Options};
use crate::tnc::Tnc;
use std::ffi::{CStr, c_char};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
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
        if !self.drive.is_finite() || !(-30.0..=0.0).contains(&self.drive) {
            return Err("TX drive must be -30 to 0 dB.".into());
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
        if matches!(self.ptt, Some(PttSpec::Cm108 { .. })) {
            return Err("Use serial RTS/DTR, rigctld, or VOX on macOS.".into());
        }
        audio::coreaudio::check(&self.playback, self.ptt.is_some()).map_err(|e| e.to_string())
    }

    fn with_saved(&self, saved: &Pairs) -> Result<Self, String> {
        let mut next = self.clone();
        for (key, value) in saved {
            next.set(key, value)?;
        }
        next.validate()?;
        Ok(next)
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

fn devices() -> std::io::Result<Devices> {
    let (capture, playback) = audio::coreaudio::list()?;
    let map = |v: Vec<(String, String)>| v.into_iter().map(|(name, desc)| Device { name, desc }).collect();
    Ok(Devices { backend: "coreaudio", capture: map(capture), playback: map(playback) })
}

fn run(base: Config, dir: PathBuf, stop: Arc<AtomicBool>, status: Arc<Mutex<Status>>) -> Result<(), String> {
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
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
        let mut io =
            audio::coreaudio::CoreAudio::open(&cfg.capture, &cfg.playback, cfg.ptt.is_some(), cfg.ptt.is_none())
                .map_err(|e| e.to_string())?;
        let host = Host::bind("127.0.0.1", cfg.port).map_err(|e| format!("Command/data ports: {e}"))?;
        // Queue buffers contribute 30 ms in addition to the configured PTT lead time.
        let opts = Options { drive_db: cfg.drive, tx_latency: (cfg.timing.lead.as_millis() as i64 * 48) + 1440 };
        let result = if let Some(bw) = cfg.fm {
            let mut tnc = Tnc::new(station::fm::Station::new(fm::Config::new(bw, &cfg.call), opts), host);
            serve(
                &cfg,
                &mut io,
                &mut tnc,
                settings,
                Workspace { stop: &stop, status: &status, token: &token, port: &mut ui_port },
            )
        } else {
            let mut tnc = Tnc::new(station::hf::Station::new(hf::Config::new(cfg.hf, &cfg.call), opts), host);
            serve(
                &cfg,
                &mut io,
                &mut tnc,
                settings,
                Workspace { stop: &stop, status: &status, token: &token, port: &mut ui_port },
            )
        }?;
        if !result || stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        update(&status, 1, "Applying modem settings…".into());
    }
}

struct Workspace<'a> {
    stop: &'a Arc<AtomicBool>,
    status: &'a Mutex<Status>,
    token: &'a str,
    port: &'a mut u16,
}

fn serve<M: Modem>(
    cfg: &Config,
    io: &mut dyn Audio,
    tnc: &mut Tnc<M>,
    settings: Arc<Settings>,
    workspace: Workspace<'_>,
) -> Result<bool, String> {
    let Workspace { stop, status, token, port: ui_port } = workspace;
    let mut ui = None;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(port) = cfg.kiss_port {
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
            Access { control: true, token: Some(token.into()), settings: Some(settings), hosts: Vec::new() },
        )
        .map_err(|e| e.to_string())?;
        *ui_port = server.addr.port();
        let url = format!("http://127.0.0.1:{}/?token={token}", *ui_port);
        ui = Some(server);
        tnc.set_telemetry(telemetry);
        tnc.set_echo(false);
        let mut ptt = cfg
            .ptt
            .as_ref()
            .map(|spec| spec.open().map(|dev| PttCtl::spawn(dev, cfg.timing, stop.clone())))
            .transpose()
            .map_err(|e| e.to_string())?;
        update(status, 2, url);
        let mut input = [0i16; audio::BLOCK];
        let mut output = [0i16; audio::BLOCK];
        let mut rx = [0.0; audio::BLOCK];
        let mut tx = [0.0; audio::BLOCK];
        let mut delay = audio::TxAudio::new(cfg.timing.lead.as_millis() as usize * 48);
        let pumped = (|| {
            while !stop.load(Ordering::SeqCst) {
                io.read(&mut input).map_err(|e| e.to_string())?;
                for (x, sample) in rx.iter_mut().zip(input) {
                    *x = audio::sample(sample);
                }
                CAN_CONFIGURE.store(tnc.station().idle() && !tnc.keyed() && !tnc.station().tuning(), Ordering::SeqCst);
                let forced = cfg.ptt.is_none() || ptt.as_ref().is_some_and(PttCtl::forced_off);
                tnc.force_ptt_off(forced);
                tnc.process(&rx, &mut tx);
                let keyed = tnc.keyed();
                if keyed {
                    if let Some(ptt) = &mut ptt {
                        ptt.set(true).map_err(|e| e.to_string())?;
                    }
                }
                delay.process(&mut tx, forced);
                for (sample, x) in output.iter_mut().zip(tx) {
                    *sample = audio::quantize(x);
                }
                if cfg.ptt.is_some() {
                    io.write(&output).map_err(|e| e.to_string())?;
                }
                if !keyed {
                    if let Some(ptt) = &mut ptt {
                        ptt.set(false).map_err(|e| e.to_string())?;
                    }
                }
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
    CAN_CONFIGURE.store(true, Ordering::SeqCst);
    tnc.shutdown();
    if let Some(ui) = ui {
        ui.shutdown();
    }
    match result {
        Ok(result) => result,
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
            let host = Host::bind("127.0.0.1", 0).unwrap();
            let mut tnc =
                Tnc::new(station::hf::Station::new(hf::Config::new(cfg.hf, &cfg.call), Options::default()), host);
            let mut input = SimulatedInput(reads);
            let mut port = port;
            let restart = serve(
                &cfg,
                &mut input,
                &mut tnc,
                settings,
                Workspace { stop: &stop, status: &status, token: "embedded-test-token", port: &mut port },
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
    fn config_rejects_invalid_identity_ports_and_radio_defaults() {
        assert!(Config::parse("call=N0CALL").is_err());
        assert!(Config::parse("call=W1AW\nport=65535").is_err());
        assert!(Config::parse("call=W1AW\nport=8100").is_err());
        assert!(Config::parse("call=W1AW\nptt=vox").is_err());
        assert!(Config::parse("call=W1AW\ndrive-db=NaN").is_err());
        assert!(Config::parse("call=W1AW\nptt=cm108:/dev/fake").is_err());
    }
}
