#![allow(dead_code)]

use std::path::PathBuf;
use xara_testkit::hex;
use xara_testkit::json::Value;

pub fn vectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vectors")
}

pub fn load_doc(name: &str) -> Value {
    let path = vectors_dir().join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    xara_testkit::json::from_str(&text).expect("vector JSON")
}

pub fn load(name: &str) -> Value {
    load_doc(name)["content"].clone()
}

pub fn captures_dir() -> PathBuf {
    std::env::var_os("XARA_CAPTURES")
        .map(PathBuf::from)
        .expect("set XARA_CAPTURES to the directory that holds the VARA lab captures")
}

pub fn hex_frames(v: &Value) -> Vec<Vec<u8>> {
    v.as_array().expect("array").iter().map(|h| hex::decode(h.as_str().expect("hex string")).expect("hex")).collect()
}

pub fn hex_u32(v: &Value) -> u32 {
    let s = v.as_str().expect("hex string");
    u32::from_str_radix(s.trim_start_matches("0x"), 16).expect("hex")
}

pub fn u32s(v: &Value) -> Vec<u32> {
    v.as_array().expect("array").iter().map(|x| x.as_u64().expect("uint") as u32).collect()
}

pub fn words(v: &Value) -> Vec<u32> {
    v.as_str().expect("string").split_whitespace().map(|w| w.parse().expect("int")).collect()
}

pub fn bw_of(v: &Value) -> xara_phy::hf::level::Bandwidth {
    v.as_u64().expect("bw").to_string().parse().unwrap()
}

pub fn sha256_hex(b: &[u8]) -> String {
    xara_testkit::sha256::hex(b)
}

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn below(&mut self, n: u32) -> u32 {
        (self.uniform() * f64::from(n)) as u32
    }

    pub fn gauss(&mut self) -> f64 {
        let u = self.uniform().max(1e-300);
        let v = self.uniform();
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }

    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next_u64() as u8).collect()
    }
}

pub fn random_frame(rng: &mut Rng, nbytes: usize) -> Vec<u8> {
    let mut f = rng.bytes(nbytes - 2);
    f.extend(xara_phy::crc::crc16_genibus(&f).to_be_bytes());
    f
}

pub fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(items.len());
    let mut out: Vec<(usize, R)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut got = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break got };
                        got.push((i, f(item)));
                    }
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().unwrap_or_else(|e| std::panic::resume_unwind(e))).collect()
    });
    out.sort_by_key(|p| p.0);
    out.into_iter().map(|p| p.1).collect()
}

pub fn read_wav(path: &std::path::Path) -> Vec<f64> {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&b[..4], b"RIFF");
    let mut i = 12;
    let mut fmt = None;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let n = u32::from_le_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
        let body = &b[i + 8..(i + 8 + n).min(b.len())];
        if id == b"fmt " {
            let ch = u16::from_le_bytes([body[2], body[3]]) as usize;
            let rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            let bits = u16::from_le_bytes([body[14], body[15]]);
            assert_eq!((rate, bits), (48000, 16), "{}", path.display());
            fmt = Some(ch);
        } else if id == b"data" {
            let ch = fmt.expect("fmt before data");
            return body.chunks_exact(2 * ch).map(|s| f64::from(i16::from_le_bytes([s[0], s[1]])) / 32768.0).collect();
        }
        i += 8 + n + (n & 1);
    }
    panic!("{}: no data chunk", path.display())
}
