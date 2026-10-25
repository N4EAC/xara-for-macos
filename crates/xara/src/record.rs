use crate::{audio, flac, wav};
use std::fs::File;
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

pub const KEYED: u8 = 1;
pub const FORCED_OFF: u8 = 2;
pub const TUNING: u8 = 4;
const QUEUE_BLOCKS: usize = audio::RATE as usize / audio::BLOCK * 5 / 2;

pub fn ptt(keyed: bool, forced_off: bool, tuning: bool) -> u8 {
    [(keyed, KEYED), (forced_off, FORCED_OFF), (tuning, TUNING)].iter().filter(|b| b.0).fold(0, |a, b| a | b.1)
}

pub fn part(path: &Path, n: u32) -> PathBuf {
    if n == 0 {
        return path.to_path_buf();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    path.with_file_name(match path.extension() {
        Some(e) => format!("{stem}-{n}.{}", e.to_string_lossy()),
        None => format!("{stem}-{n}"),
    })
}

pub fn utc(secs: u64) -> String {
    let (days, s) = ((secs / 86400) as i64, secs % 86400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

struct Block {
    rx: Vec<i16>,
    tx: Vec<i16>,
    ptt: u8,
}

enum Sink {
    Flac(flac::Writer<BufWriter<File>>),
    Wav(wav::Writer<BufWriter<File>>),
}

impl Sink {
    fn write(&mut self, b: &Block, scratch: &mut Vec<i16>) -> io::Result<bool> {
        let p = i16::from(b.ptt) << 8;
        scratch.clear();
        match self {
            Sink::Flac(w) => {
                scratch.resize(b.rx.len(), p);
                w.write([&b.rx, &b.tx, scratch]).map(|()| true)
            }
            Sink::Wav(w) => {
                scratch.extend(b.rx.iter().zip(&b.tx).flat_map(|(&r, &t)| [r, t, p]));
                w.write(scratch)
            }
        }
    }

    fn finish(self) -> io::Result<()> {
        match self {
            Sink::Flac(w) => w.finish().map(drop),
            Sink::Wav(w) => w.finish().map(drop),
        }
    }
}

pub struct Recorder {
    queue: Option<SyncSender<Block>>,
    thread: Option<JoinHandle<()>>,
    dropped: u64,
}

impl Recorder {
    pub fn open(path: &Path, mode: &str) -> io::Result<Self> {
        let out = BufWriter::new(File::create(path)?);
        let sink = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav")) {
            Sink::Wav(wav::Writer::new(out, flac::CHANNELS as u16, audio::RATE)?)
        } else {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
            Sink::Flac(flac::Writer::new(out, &[format!("START={}", utc(now)), format!("MODE={mode}")])?)
        };
        let (queue, blocks) = mpsc::sync_channel(QUEUE_BLOCKS);
        let name = path.display().to_string();
        let thread = thread::Builder::new().name("record".into()).spawn(move || drain(sink, blocks, &name))?;
        Ok(Self { queue: Some(queue), thread: Some(thread), dropped: 0 })
    }

    pub fn push(&mut self, rx: &[i16], tx: &[i16], ptt: u8) {
        let Some(q) = &self.queue else { return };
        match q.try_send(Block { rx: rx.to_vec(), tx: tx.to_vec(), ptt }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.dropped += 1;
                if self.dropped.is_power_of_two() {
                    say!("record: writer is behind, {} blocks dropped", self.dropped);
                }
            }
            Err(TrySendError::Disconnected(_)) => self.queue = None,
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.queue = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn drain(mut sink: Sink, blocks: Receiver<Block>, name: &str) {
    let (mut scratch, mut samples) = (Vec::new(), 0u64);
    for b in blocks {
        match sink.write(&b, &mut scratch) {
            Ok(true) => samples += b.rx.len() as u64,
            Ok(false) => {
                say!("record: {name} is full at 4 GB, recording stopped");
                break;
            }
            Err(e) => {
                say!("record: {name}: {e}, recording stopped");
                break;
            }
        }
    }
    match sink.finish() {
        Ok(()) => say!("record: {samples} samples in {name}"),
        Err(e) => say!("record: {name}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ptt_bits_are_keyed_forced_off_tuning() {
        assert_eq!(ptt(false, false, false), 0);
        assert_eq!(ptt(true, false, true), 5);
        assert_eq!(ptt(false, true, false), 2);
        assert_eq!(ptt(true, true, true), 7);
    }

    #[test]
    fn restarts_record_to_numbered_parts() {
        assert_eq!(part(Path::new("/tmp/a.flac"), 0), Path::new("/tmp/a.flac"));
        assert_eq!(part(Path::new("/tmp/a.flac"), 2), Path::new("/tmp/a-2.flac"));
        assert_eq!(part(Path::new("cap"), 1), Path::new("cap-1"));
    }

    #[test]
    fn start_times_are_rfc3339_utc() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(951_825_600), "2000-02-29T12:00:00Z");
        assert_eq!(utc(1_792_800_000), "2026-10-24T00:00:00Z");
        assert_eq!(utc(4_107_542_399), "2100-02-28T23:59:59Z");
    }

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("xara-record-{}-{name}", std::process::id()))
    }

    fn record(path: &Path, blocks: usize) -> Vec<i16> {
        let mut r = Recorder::open(path, "HF 2300").unwrap();
        let mut rx = Vec::new();
        for i in 0..blocks {
            let x: Vec<i16> = (0..audio::BLOCK).map(|j| ((i * 7919 + j * 104_729) % 65536) as u16 as i16).collect();
            let tx: Vec<i16> = x.iter().map(|v| v / 3).collect();
            r.push(&x, &tx, ptt(i % 3 == 0, false, i == 5));
            rx.extend(x);
        }
        drop(r);
        rx
    }

    #[test]
    fn flac_and_wav_records_carry_rx_tx_and_ptt() {
        let p = scratch("a.flac");
        let rx = record(&p, 20);
        let c = flac::read(&p).unwrap();
        assert_eq!(c.channels[0], rx);
        assert!(c.channels[1].iter().zip(&rx).all(|(&t, &r)| t == r / 3));
        assert_eq!(c.channels[2][5 * audio::BLOCK], 0x0400);
        assert_eq!(c.channels[2][6 * audio::BLOCK], 0x0100);
        assert_eq!(c.channels[2][audio::BLOCK], 0);
        assert!(c.tags.iter().any(|t| t == "MODE=HF 2300") && c.tags.iter().any(|t| t.starts_with("START=")));
        std::fs::remove_file(&p).unwrap();
        let p = scratch("b.WAV");
        let rx = record(&p, 20);
        let w = wav::read(&p).unwrap();
        assert_eq!(w.samples, rx.into_iter().map(audio::sample).collect::<Vec<_>>());
        let b = std::fs::read(&p).unwrap();
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()) as usize, 20 * audio::BLOCK * 6);
        assert_eq!(i16::from_le_bytes([b[44 + 4], b[44 + 5]]), 0x0100);
        std::fs::remove_file(&p).unwrap();
    }
}
