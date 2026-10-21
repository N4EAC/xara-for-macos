use std::io;

pub struct Wav {
    pub rate: u32,
    pub samples: Vec<f64>,
}

fn bad(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_string())
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

pub fn s16(s: &[u8]) -> f64 {
    f64::from(i16::from_le_bytes([s[0], s[1]])) / 32768.0
}

pub fn read(path: &std::path::Path) -> io::Result<Wav> {
    parse(&std::fs::read(path)?)
}

fn parse(b: &[u8]) -> io::Result<Wav> {
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err(bad("not a RIFF/WAVE file"));
    }
    let mut i = 12;
    let mut fmt = None;
    while b.len().saturating_sub(i) >= 8 {
        let id = &b[i..i + 4];
        let n = usize::try_from(u32_at(b, i + 4)).map_err(|_| bad("chunk too large"))?;
        let body = &b[i + 8..(i + 8).saturating_add(n).min(b.len())];
        if id == b"fmt " {
            if body.len() < 16 {
                return Err(bad("short fmt chunk"));
            }
            let mut tag = u16_at(body, 0);
            let ch = usize::from(u16_at(body, 2));
            let rate = u32_at(body, 4);
            let bits = u16_at(body, 14);
            if tag == 0xFFFE && body.len() >= 26 {
                tag = u16_at(body, 24);
            }
            fmt = Some((tag, ch, rate, bits));
        } else if id == b"data" {
            let (tag, ch, rate, bits) = fmt.ok_or_else(|| bad("data before fmt"))?;
            let w = usize::from(bits / 8);
            if ch == 0 || w == 0 {
                return Err(bad("bad fmt chunk"));
            }
            if rate == 0 {
                return Err(bad("sample rate 0"));
            }
            let sample = |s: &[u8]| -> Option<f64> {
                Some(match (tag, bits) {
                    (1, 8) => (f64::from(s[0]) - 128.0) / 128.0,
                    (1, 16) => s16(s),
                    (1, 24) => f64::from(i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) / 8_388_608.0,
                    (1, 32) => f64::from(i32::from_le_bytes([s[0], s[1], s[2], s[3]])) / 2_147_483_648.0,
                    (3, 32) => f64::from(f32::from_le_bytes([s[0], s[1], s[2], s[3]])),
                    (3, 64) => f64::from_le_bytes(s[..8].try_into().unwrap()),
                    _ => return None,
                })
            };
            let frame = w.checked_mul(ch).ok_or_else(|| bad("bad fmt chunk"))?;
            let samples: Option<Vec<f64>> = body.chunks_exact(frame).map(|f| sample(&f[..w])).collect();
            let samples = samples.ok_or_else(|| bad(&format!("unsupported WAV format {tag}, {bits} bits")))?;
            return Ok(Wav { rate, samples });
        }
        match n.checked_add(8 + (n & 1)).and_then(|step| i.checked_add(step)) {
            Some(next) => i = next,
            None => break,
        }
    }
    Err(bad("no data chunk"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: &[u8; 4], size: u32, body: &[u8]) -> Vec<u8> {
        [&id[..], &size.to_le_bytes(), body].concat()
    }

    fn fmt(tag: u16, ch: u16, rate: u32, bits: u16) -> Vec<u8> {
        let body = [tag.to_le_bytes(), ch.to_le_bytes()]
            .concat()
            .into_iter()
            .chain(rate.to_le_bytes())
            .chain([0; 6])
            .chain(bits.to_le_bytes())
            .collect::<Vec<u8>>();
        chunk(b"fmt ", 16, &body)
    }

    fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
        [&b"RIFF\0\0\0\0WAVE"[..], &chunks.concat()].concat()
    }

    fn err(b: &[u8]) -> String {
        parse(b).err().expect("a crafted header must be rejected").to_string()
    }

    #[test]
    fn reads_a_plain_file() {
        let pcm: Vec<u8> = [0i16, 16384, -32768].iter().flat_map(|v| v.to_le_bytes()).collect();
        let w = parse(&riff(&[fmt(1, 1, 12000, 16), chunk(b"data", 6, &pcm)])).unwrap();
        assert_eq!((w.rate, w.samples), (12000, vec![0.0, 0.5, -1.0]));
    }

    #[test]
    fn crafted_headers_are_errors_not_panics() {
        let pcm = [0u8; 8];
        assert!(err(b"RIFF").contains("not a RIFF"));
        assert!(err(&riff(&[])).contains("no data chunk"));
        assert!(err(&riff(&[chunk(b"data", 8, &pcm)])).contains("data before fmt"));
        assert!(err(&riff(&[chunk(b"fmt ", 16, &[1, 0, 1])])).contains("short fmt"));
        assert!(err(&riff(&[chunk(b"fmt ", u32::MAX, &[1, 0, 1, 0])])).contains("short fmt"));
        assert!(err(&riff(&[chunk(b"LIST", u32::MAX, &pcm), chunk(b"data", 8, &pcm)])).contains("no data chunk"));
        assert!(err(&riff(&[chunk(b"LIST", u32::MAX - 8, &pcm)])).contains("no data chunk"));
        assert!(err(&riff(&[fmt(1, 0, 12000, 16), chunk(b"data", 8, &pcm)])).contains("bad fmt"));
        assert!(err(&riff(&[fmt(1, 1, 12000, 4), chunk(b"data", 8, &pcm)])).contains("bad fmt"));
        assert!(err(&riff(&[fmt(1, 1, 0, 16), chunk(b"data", 8, &pcm)])).contains("sample rate 0"));
        assert!(err(&riff(&[fmt(7, 1, 8000, 8), chunk(b"data", 8, &pcm)])).contains("unsupported"));
        let mut odd = riff(&[fmt(1, 1, 8000, 8), chunk(b"data", u32::MAX, &pcm)]);
        assert_eq!(parse(&odd).unwrap().samples.len(), 8);
        odd.truncate(odd.len() - 8);
        assert!(parse(&odd).unwrap().samples.is_empty());
        assert_eq!(parse(&riff(&[fmt(1, 65535, 8000, 64000), chunk(b"data", 8, &pcm)])).unwrap().samples.len(), 0);
    }
}
