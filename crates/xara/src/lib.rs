#[macro_use]
pub mod log;
pub mod audio;
pub mod cli;
pub mod flac;
pub mod monitor;
pub mod ptt;
pub mod record;
pub mod resample;
pub mod signals;
pub mod station;
pub mod tnc;
pub mod wav;

pub const DEFAULT_ADDR: &str = "127.0.0.1";
pub const HF_SPAN_HZ: u32 = 3000;
pub const FM_SPAN_HZ: u32 = 6000;
