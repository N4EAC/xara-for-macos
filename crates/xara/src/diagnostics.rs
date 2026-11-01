//! Bounded, opt-in-at-initialization operational diagnostics. Never receives payload/audio buffers.
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

pub const LIMIT: usize = 1_048_576;
const HEADER: &str = "XARA operational diagnostics\nAutomatic local logging: current + previous file, 1 MiB each. Queue bounded to 256 records.\nIncludes OS/build, audio device names/UIDs, channels/rates/latency, errors, timing and numeric signal summaries.\nNo audio recordings, message payloads, raw host commands, callsign configuration or UI authentication tokens. Nothing is uploaded automatically.\n";
static LOGGER: OnceLock<Logger> = OnceLock::new();
struct Logger {
    tx: SyncSender<Message>,
    dropped: AtomicUsize,
    error: Arc<Mutex<String>>,
}
enum Message {
    Line(String),
    Snapshot(SyncSender<io::Result<String>>),
}
struct Store {
    dir: PathBuf,
    file: File,
    bytes: usize,
    limit: usize,
}
fn regular(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if !m.file_type().is_symlink() && m.is_file() && m.nlink() == 1 => Ok(()),
        Ok(_) => Err(io::Error::other("diagnostic log must be a regular, unlinked file")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
use std::os::unix::fs::MetadataExt;
fn open(path: &Path, truncate: bool) -> io::Result<File> {
    regular(path)?;
    let f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(truncate)
        .append(!truncate)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let m = f.metadata()?;
    if !m.is_file() || m.nlink() != 1 {
        return Err(io::Error::other("unsafe diagnostic log"));
    }
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(f)
}
impl Store {
    fn new(dir: &Path, limit: usize) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let m = std::fs::symlink_metadata(dir)?;
        if !m.is_dir() || m.file_type().is_symlink() || m.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::other("unsafe diagnostic directory"));
        }
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        let current = dir.join("current.log");
        let previous = dir.join("previous.log");
        regular(&current)?;
        regular(&previous)?;
        if current.exists() {
            std::fs::rename(&current, &previous)?;
        }
        if previous.exists() {
            let f = open(&previous, false)?;
            // A file from a different version/user must not break the retention bound.
            if f.metadata()?.len() > limit as u64 {
                f.set_len(limit as u64)?;
            }
        }
        Ok(Self { dir: dir.into(), file: open(&current, true)?, bytes: 0, limit })
    }
    fn write(&mut self, text: &str) -> io::Result<()> {
        if self.bytes + text.len() > self.limit {
            self.file.flush()?;
            let previous = self.dir.join("previous.log");
            regular(&previous)?;
            std::fs::rename(self.dir.join("current.log"), previous)?;
            self.file = open(&self.dir.join("current.log"), true)?;
            self.bytes = 0;
        }
        self.file.write_all(text.as_bytes())?;
        self.bytes += text.len();
        Ok(())
    }
    fn snapshot(&mut self) -> io::Result<String> {
        self.file.flush()?;
        let mut out = String::from(HEADER);
        for name in ["previous.log", "current.log"] {
            let path = self.dir.join(name);
            regular(&path)?;
            match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&path) {
                Ok(f) => {
                    let mut bytes = Vec::new();
                    f.take(self.limit as u64).read_to_end(&mut bytes)?;
                    out.push_str(&format!("\n--- {name} ---\n"));
                    out.push_str(&String::from_utf8_lossy(&bytes));
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }
}

pub fn init(dir: &Path) -> io::Result<()> {
    if LOGGER.get().is_some() {
        return Ok(());
    }
    let mut store = Store::new(dir, LIMIT)?;
    let (tx, rx) = mpsc::sync_channel(256);
    let error = Arc::new(Mutex::new(String::new()));
    let health = error.clone();
    std::thread::Builder::new().name("xara-diagnostics".into()).spawn(move || {
        while let Ok(message) = rx.recv() {
            let result = match message {
                Message::Line(line) => store.write(&line),
                Message::Snapshot(reply) => {
                    let _ = reply.send(store.snapshot());
                    Ok(())
                }
            };
            if let Err(e) = result {
                *health.lock().unwrap_or_else(|e| e.into_inner()) = e.to_string();
            }
        }
    })?;
    LOGGER
        .set(Logger { tx, dropped: AtomicUsize::new(0), error })
        .map_err(|_| io::Error::other("diagnostics already initialized"))?;
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Do not persist the panic payload: it may contain user data.
        record(format_args!(
            "panic location={:?} thread={:?} backtrace={}",
            info.location(),
            std::thread::current().name(),
            std::backtrace::Backtrace::force_capture()
        ));
        previous_hook(info);
    }));
    record(format_args!(
        "session_start modem={} arch={} retention_bytes={}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::ARCH,
        2 * LIMIT
    ));
    Ok(())
}

pub fn record(args: fmt::Arguments) {
    let Some(log) = LOGGER.get() else { return };
    let text = args.to_string();
    let text: String = text.chars().take(4096).map(|c| if c.is_control() { ' ' } else { c }).collect();
    let text = format!("{} {text}\n", crate::log::stamp(SystemTime::now()));
    if log.tx.try_send(Message::Line(text)).is_err() {
        log.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn snapshot() -> io::Result<String> {
    let log = LOGGER.get().ok_or_else(|| io::Error::other("diagnostic logging is not initialized"))?;
    let (tx, rx) = mpsc::sync_channel(1);
    log.tx.try_send(Message::Snapshot(tx)).map_err(|_| io::Error::other("diagnostic writer is busy; retry export"))?;
    let text = rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| io::Error::other("diagnostic writer did not respond"))??;
    Ok(format!(
        "{text}\nLogging health: dropped_records={} last_write_error={:?}\n",
        log.dropped.load(Ordering::Relaxed),
        *log.error.lock().unwrap_or_else(|e| e.into_inner())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotation_retention_permissions_and_restart() {
        let root = std::env::temp_dir().join(format!("xara-diagnostic-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut s = Store::new(&root, 64).unwrap();
        s.write(&"a".repeat(50)).unwrap();
        s.write(&"b".repeat(50)).unwrap();
        s.write(&"c".repeat(50)).unwrap();
        let out = s.snapshot().unwrap();
        assert!(!out.contains(&"a".repeat(50)));
        assert!(out.contains(&"b".repeat(50)) && out.contains(&"c".repeat(50)));
        assert_eq!(std::fs::metadata(root.join("current.log")).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(&root).unwrap().permissions().mode() & 0o777, 0o700);
        drop(s);
        let mut s = Store::new(&root, 64).unwrap();
        assert!(s.snapshot().unwrap().contains(&"c".repeat(50)));
        drop(s);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn writer_exports_operational_events_without_panic_payloads() {
        let root = std::env::temp_dir().join(format!("xara-diagnostic-pipeline-{}", std::process::id()));
        init(&root).unwrap();
        record(format_args!("probe\nforged_line\t{}", "é".repeat(5000)));
        let _ = std::panic::catch_unwind(|| panic!("DO_NOT_SAVE_PAYLOAD_0129"));
        let report = snapshot().unwrap();
        assert!(report.contains("session_start modem="));
        assert!(report.contains("probe forged_line "));
        assert!(!report.contains("\nforged_line"));
        assert!(report.contains("panic location="));
        assert!(!report.contains("DO_NOT_SAVE_PAYLOAD_0129"));
        assert!(report.contains("Logging health: dropped_records="));
        assert!(std::fs::metadata(root.join("current.log")).unwrap().len() <= LIMIT as u64);
    }

    #[test]
    fn unsafe_log_paths_are_rejected() {
        let root = std::env::temp_dir().join(format!("xara-diagnostic-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("target");
        std::fs::write(&target, "untouched").unwrap();
        std::os::unix::fs::symlink(&target, root.join("current.log")).unwrap();
        assert!(Store::new(&root, 64).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "untouched");
        std::fs::remove_file(root.join("current.log")).unwrap();
        std::fs::hard_link(&target, root.join("previous.log")).unwrap();
        assert!(Store::new(&root, 64).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
