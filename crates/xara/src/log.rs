use std::fmt;
use std::io::Write;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

static NAME: OnceLock<&'static str> = OnceLock::new();
static QUIET: AtomicBool = AtomicBool::new(false);

pub fn init(name: &'static str, quiet: bool) {
    let _ = NAME.set(name);
    QUIET.store(quiet, Ordering::Relaxed);
}

pub fn write(info: bool, args: fmt::Arguments) {
    if info && QUIET.load(Ordering::Relaxed) {
        return;
    }
    let _ = writeln!(std::io::stderr().lock(), "{}: {args}", NAME.get().copied().unwrap_or("xara"));
}

#[macro_export]
macro_rules! say {
    ($($a:tt)*) => {
        $crate::log::write(false, format_args!($($a)*))
    };
}

#[macro_export]
macro_rules! info {
    ($($a:tt)*) => {
        $crate::log::write(true, format_args!($($a)*))
    };
}
