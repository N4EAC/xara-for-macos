use std::io::{self, Seek, SeekFrom, Write};

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
    if b.starts_with(b"fLaC") {
        let c = crate::flac::parse(b)?;
        let [rx, ..] = c.channels;
        return Ok(Wav { rate: c.rate, samples: rx.into_iter().map(crate::audio::sample).collect() });
    }
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

pub struct Writer<W: Write + Seek> {
    out: W,
    bytes: u32,
}

impl<W: Write + Seek> Writer<W> {
    pub fn new(mut out: W, channels: u16, rate: u32) -> io::Result<Self> {
        let frame = u32::from(channels) * 2;
        let mut h = b"RIFF\xff\xff\xff\xffWAVEfmt \x10\0\0\0\x01\0".to_vec();
        h.extend(channels.to_le_bytes());
        h.extend(rate.to_le_bytes());
        h.extend((rate * frame).to_le_bytes());
        h.extend((frame as u16).to_le_bytes());
        h.extend(16u16.to_le_bytes());
        h.extend(b"data\xff\xff\xff\xff");
        out.write_all(&h)?;
        Ok(Self { out, bytes: 0 })
    }

    pub fn write(&mut self, interleaved: &[i16]) -> io::Result<bool> {
        let n = u32::try_from(interleaved.len() * 2).ok();
        let Some(bytes) = n.and_then(|n| self.bytes.checked_add(n)).filter(|&b| b <= u32::MAX - 36) else {
            return Ok(false);
        };
        let pcm: Vec<u8> = interleaved.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.out.write_all(&pcm)?;
        self.bytes = bytes;
        Ok(true)
    }

    pub fn finish(mut self) -> io::Result<W> {
        self.out.seek(SeekFrom::Start(4))?;
        self.out.write_all(&(36 + self.bytes).to_le_bytes())?;
        self.out.seek(SeekFrom::Start(40))?;
        self.out.write_all(&self.bytes.to_le_bytes())?;
        self.out.seek(SeekFrom::End(0))?;
        self.out.flush()?;
        Ok(self.out)
    }
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
    fn a_xara_flac_capture_reads_as_its_rx_channel() {
        let mut w = crate::flac::Writer::new(io::Cursor::new(Vec::new()), &[]).unwrap();
        w.write([&[0, 16384, -32768], &[1, 2, 3], &[0; 3]]).unwrap();
        let w = parse(&w.finish().unwrap().into_inner()).unwrap();
        assert_eq!((w.rate, w.samples), (48000, vec![0.0, 0.5, -1.0]));
    }

    #[test]
    fn the_writer_patches_sizes_on_finish() {
        let mut w = Writer::new(io::Cursor::new(Vec::new()), 3, 48000).unwrap();
        assert!(w.write(&[0, 1, 2, 16384, -32768, 3]).unwrap());
        let open = w.out.get_ref().clone();
        assert_eq!((&open[4..8], &open[40..44]), (&[0xFF; 4][..], &[0xFF; 4][..]));
        assert_eq!(parse(&open).unwrap().samples, [0.0, 0.5]);
        let b = w.finish().unwrap().into_inner();
        let mut want = b"RIFF\x30\0\0\0WAVEfmt \x10\0\0\0\x01\0\x03\0".to_vec();
        want.extend(48000u32.to_le_bytes());
        want.extend((48000u32 * 6).to_le_bytes());
        want.extend(b"\x06\0\x10\0data\x0c\0\0\0");
        assert_eq!(b[..44], want[..]);
        assert_eq!(b.len(), 56);
        let w = parse(&b).unwrap();
        assert_eq!((w.rate, w.samples), (48000, vec![0.0, 0.5]));
    }

    #[test]
    fn the_writer_stops_short_of_4_gb() {
        let mut w = Writer::new(io::Cursor::new(Vec::new()), 3, 48000).unwrap();
        w.bytes = u32::MAX - 36 - 6;
        assert!(w.write(&[0; 3]).unwrap());
        assert!(!w.write(&[0; 3]).unwrap());
        assert_eq!(w.bytes, u32::MAX - 36);
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
