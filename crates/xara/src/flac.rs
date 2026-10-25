use std::io::{self, Seek, SeekFrom, Write};

const RATE: u32 = crate::audio::RATE;
const _: () = assert!(RATE == 48000);
pub const CHANNELS: usize = 3;
const BLOCKSIZE: usize = 4096;
const VENDOR: &str = concat!("xara ", env!("CARGO_PKG_VERSION"));
const MARK: &str = "XARA_CAPTURE=1";
const MAX_ORDER: usize = 4;
const MAX_PARTITION_ORDER: u32 = 8;
const ESCAPE: u8 = 15;
const TOTAL_AT: u64 = 18;

#[derive(Debug)]
pub struct Capture {
    pub rate: u32,
    pub channels: [Vec<i16>; CHANNELS],
    pub tags: Vec<String>,
}

fn crc8(b: &[u8]) -> u8 {
    b.iter().fold(0u8, |mut c, &x| {
        c ^= x;
        for _ in 0..8 {
            c = if c & 0x80 != 0 { (c << 1) ^ 0x07 } else { c << 1 };
        }
        c
    })
}

fn crc16(b: &[u8]) -> u16 {
    b.iter().fold(0u16, |mut c, &x| {
        c ^= u16::from(x) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 { (c << 1) ^ 0x8005 } else { c << 1 };
        }
        c
    })
}

const MD5_SHIFT: [[u32; 4]; 4] = [[7, 12, 17, 22], [5, 9, 14, 20], [4, 11, 16, 23], [6, 10, 15, 21]];

struct Md5 {
    s: [u32; 4],
    buf: Vec<u8>,
    len: u64,
}

impl Md5 {
    fn new() -> Self {
        Self { s: [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476], buf: Vec::new(), len: 0 }
    }

    fn update(&mut self, b: &[u8]) {
        self.len += b.len() as u64;
        self.buf.extend_from_slice(b);
        let n = self.buf.len() / 64 * 64;
        for c in self.buf[..n].chunks_exact(64) {
            md5_block(&mut self.s, c);
        }
        self.buf.drain(..n);
    }

    fn samples(&mut self, ch: &[Vec<i16>; CHANNELS], n: usize) {
        let b: Vec<u8> = (0..n).flat_map(|i| ch.iter().flat_map(move |c| c[i].to_le_bytes())).collect();
        self.update(&b);
    }

    fn digest(mut self) -> [u8; 16] {
        let bits = self.len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.buf.len() != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_le_bytes());
        let mut d = [0; 16];
        for (o, v) in d.chunks_exact_mut(4).zip(self.s) {
            o.copy_from_slice(&v.to_le_bytes());
        }
        d
    }
}

fn md5_block(s: &mut [u32; 4], c: &[u8]) {
    static K: std::sync::OnceLock<[u32; 64]> = std::sync::OnceLock::new();
    let k = K.get_or_init(|| std::array::from_fn(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32));
    let m: [u32; 16] = std::array::from_fn(|i| u32::from_le_bytes(c[4 * i..4 * i + 4].try_into().unwrap()));
    let [mut a, mut b, mut x, mut d] = *s;
    for i in 0..64 {
        let (f, g) = match i / 16 {
            0 => ((b & x) | (!b & d), i),
            1 => ((d & b) | (!d & x), (5 * i + 1) % 16),
            2 => (b ^ x ^ d, (3 * i + 5) % 16),
            _ => (x ^ (b | !d), 7 * i % 16),
        };
        let t = a.wrapping_add(f).wrapping_add(k[i]).wrapping_add(m[g]).rotate_left(MD5_SHIFT[i / 16][i % 4]);
        (a, d, x, b) = (d, x, b, b.wrapping_add(t));
    }
    for (v, n) in s.iter_mut().zip([a, b, x, d]) {
        *v = v.wrapping_add(n);
    }
}

#[derive(Default)]
struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn put(&mut self, v: u64, bits: u32) {
        if bits == 0 {
            return;
        }
        self.acc = (self.acc << bits) | (v & ((1u64 << bits) - 1));
        self.n += bits;
        while self.n >= 8 {
            self.n -= 8;
            self.out.push((self.acc >> self.n) as u8);
        }
        self.acc &= (1u64 << self.n) - 1;
    }

    fn zeros(&mut self, mut q: u64) {
        while q > 0 {
            let k = q.min(32) as u32;
            self.put(0, k);
            q -= u64::from(k);
        }
    }

    fn align(&mut self) {
        if self.n > 0 {
            self.put(0, 8 - self.n);
        }
    }
}

fn utf8(w: &mut Bits, v: u64) {
    if v < 0x80 {
        return w.put(v, 8);
    }
    let n = (2..=7u32).find(|&n| v < 1u64 << (5 * n + 1)).unwrap_or(7);
    w.put(((0xFF00u64 >> n) & 0xFF) | (v >> (6 * (n - 1))), 8);
    for i in (0..n - 1).rev() {
        w.put(0x80 | ((v >> (6 * i)) & 0x3F), 8);
    }
}

fn zigzag(r: i64) -> u64 {
    ((r << 1) ^ (r >> 63)) as u64
}

fn width(r: i64) -> u32 {
    match r {
        0 => 0,
        r if r > 0 => 65 - r.leading_zeros(),
        r => 65 - (!r).leading_zeros(),
    }
}

fn residual(x: &[i64], order: usize) -> Vec<i64> {
    (order..x.len())
        .map(|i| match order {
            0 => x[i],
            1 => x[i] - x[i - 1],
            2 => x[i] - 2 * x[i - 1] + x[i - 2],
            3 => x[i] - 3 * x[i - 1] + 3 * x[i - 2] - x[i - 3],
            _ => x[i] - 4 * x[i - 1] + 6 * x[i - 2] - 4 * x[i - 3] + x[i - 4],
        })
        .collect()
}

fn predict(x: &[i64], i: usize, order: usize) -> i64 {
    match order {
        0 => 0,
        1 => x[i - 1],
        2 => 2 * x[i - 1] - x[i - 2],
        3 => 3 * x[i - 1] - 3 * x[i - 2] + x[i - 3],
        _ => 4 * x[i - 1] - 6 * x[i - 2] + 4 * x[i - 3] - x[i - 4],
    }
}

#[derive(Clone)]
struct Part {
    count: u64,
    sums: [u64; ESCAPE as usize],
    width: u32,
}

impl Part {
    fn merge(&self, o: &Part) -> Part {
        let mut sums = self.sums;
        sums.iter_mut().zip(&o.sums).for_each(|(a, b)| *a += b);
        Part { count: self.count + o.count, sums, width: self.width.max(o.width) }
    }

    fn best(&self) -> (u8, u64) {
        let rice = (0..ESCAPE).map(|k| (k, self.count * (u64::from(k) + 1) + self.sums[usize::from(k)]));
        let escape = (ESCAPE, 5 + self.count * u64::from(self.width));
        rice.chain(std::iter::once(escape)).min_by_key(|&(_, c)| c).unwrap()
    }
}

struct Plan {
    order: u32,
    params: Vec<(u8, u32)>,
    bits: u64,
}

fn plan(res: &[i64], n: usize, warm: usize) -> Plan {
    let fits = |o: u32| o == 0 || (n % (1 << o) == 0 && n >> o > warm);
    let top = (0..=MAX_PARTITION_ORDER).rev().find(|&o| fits(o)).unwrap_or(0);
    let size = n >> top;
    let mut parts: Vec<Part> = (0..1usize << top)
        .map(|j| {
            let lo = (j * size).saturating_sub(warm);
            let hi = (j + 1) * size - warm;
            let mut p = Part { count: (hi - lo) as u64, sums: [0; ESCAPE as usize], width: 0 };
            for &r in &res[lo..hi] {
                let u = zigzag(r);
                p.sums.iter_mut().enumerate().for_each(|(k, s)| *s += u >> k);
                p.width = p.width.max(width(r));
            }
            p
        })
        .collect();
    let mut best: Option<Plan> = None;
    for o in (0..=top).rev() {
        if o < top {
            parts = parts.chunks(2).map(|c| c[0].merge(&c[1])).collect();
        }
        let picks: Vec<(u8, u64)> = parts.iter().map(Part::best).collect();
        let bits = 6 + picks.iter().map(|&(_, c)| 4 + c).sum::<u64>();
        let params = picks.iter().zip(&parts).map(|(&(k, _), p)| (k, p.width)).collect();
        if best.as_ref().is_none_or(|b| bits <= b.bits) {
            best = Some(Plan { order: o, params, bits });
        }
    }
    best.unwrap()
}

fn put_residual(w: &mut Bits, res: &[i64], n: usize, warm: usize, p: &Plan) {
    w.put(0, 2);
    w.put(u64::from(p.order), 4);
    let size = n >> p.order;
    for (j, &(k, wid)) in p.params.iter().enumerate() {
        let lo = (j * size).saturating_sub(warm);
        let hi = (j + 1) * size - warm;
        w.put(u64::from(k), 4);
        if k == ESCAPE {
            w.put(u64::from(wid), 5);
            res[lo..hi].iter().for_each(|&r| w.put(r as u64, wid));
        } else {
            for &r in &res[lo..hi] {
                let u = zigzag(r);
                w.zeros(u >> k);
                w.put(1, 1);
                w.put(u, u32::from(k));
            }
        }
    }
}

fn subframe(w: &mut Bits, x: &[i16]) {
    let or = x.iter().fold(0i32, |a, &v| a | i32::from(v));
    let wasted = if or == 0 { 0 } else { or.trailing_zeros().min(15) };
    let bps = 16 - wasted;
    let s: Vec<i64> = x.iter().map(|&v| i64::from(v >> wasted)).collect();
    let head = |w: &mut Bits, kind: u64| {
        w.put(kind << 1 | u64::from(wasted > 0), 8);
        if wasted > 0 {
            w.zeros(u64::from(wasted - 1));
            w.put(1, 1);
        }
    };
    if s.iter().all(|&v| v == s[0]) {
        head(w, 0);
        return w.put(s[0] as u64, bps);
    }
    let n = s.len();
    let verbatim = n as u64 * u64::from(bps);
    let best = (0..=MAX_ORDER.min(n - 1))
        .map(|order| {
            let res = residual(&s, order);
            let p = plan(&res, n, order);
            (order as u64 * u64::from(bps) + p.bits, order, res, p)
        })
        .min_by_key(|c| c.0);
    match best {
        Some((bits, order, res, p)) if bits < verbatim => {
            head(w, 8 | order as u64);
            s[..order].iter().for_each(|&v| w.put(v as u64, bps));
            put_residual(w, &res, n, order, &p);
        }
        _ => {
            head(w, 1);
            s.iter().for_each(|&v| w.put(v as u64, bps));
        }
    }
}

fn frame(number: u64, ch: &[Vec<i16>; CHANNELS], n: usize) -> Vec<u8> {
    let mut w = Bits::default();
    w.put(0xFFF8, 16);
    w.put(if n == BLOCKSIZE { 0xCA } else { 0x7A }, 8);
    w.put(0x28, 8);
    utf8(&mut w, number);
    if n != BLOCKSIZE {
        w.put(n as u64 - 1, 16);
    }
    let c = crc8(&w.out);
    w.put(u64::from(c), 8);
    ch.iter().for_each(|c| subframe(&mut w, &c[..n]));
    w.align();
    let c = crc16(&w.out);
    w.put(u64::from(c), 16);
    w.out
}

fn put_string(c: &mut Vec<u8>, t: &str) {
    c.extend((t.len() as u32).to_le_bytes());
    c.extend(t.as_bytes());
}

fn header(tags: &[String]) -> Vec<u8> {
    let mut b = b"fLaC".to_vec();
    b.extend([0, 0, 0, 34]);
    let bs = (BLOCKSIZE as u16).to_be_bytes();
    b.extend(bs.iter().chain(&bs));
    b.extend([0; 6]);
    let packed = (u64::from(RATE) << 44) | ((CHANNELS as u64 - 1) << 41) | (15 << 36);
    b.extend(packed.to_be_bytes());
    b.extend([0; 16]);
    let all: Vec<&str> = [MARK, "CHANNELS=rx,tx,ptt"].into_iter().chain(tags.iter().map(String::as_str)).collect();
    let mut c = Vec::new();
    put_string(&mut c, VENDOR);
    c.extend((all.len() as u32).to_le_bytes());
    all.into_iter().for_each(|t| put_string(&mut c, t));
    b.push(0x84);
    b.extend(&(c.len() as u32).to_be_bytes()[1..]);
    b.extend(c);
    b
}

pub struct Writer<W: Write + Seek> {
    out: W,
    buf: [Vec<i16>; CHANNELS],
    frames: u64,
    total: u64,
    md5: Md5,
}

impl<W: Write + Seek> Writer<W> {
    pub fn new(mut out: W, tags: &[String]) -> io::Result<Self> {
        out.write_all(&header(tags))?;
        Ok(Self { out, buf: Default::default(), frames: 0, total: 0, md5: Md5::new() })
    }

    pub fn write(&mut self, ch: [&[i16]; CHANNELS]) -> io::Result<()> {
        for (b, c) in self.buf.iter_mut().zip(ch) {
            b.extend_from_slice(c);
        }
        while self.buf[0].len() >= BLOCKSIZE {
            self.emit(BLOCKSIZE)?;
        }
        Ok(())
    }

    fn emit(&mut self, n: usize) -> io::Result<()> {
        self.out.write_all(&frame(self.frames, &self.buf, n))?;
        self.md5.samples(&self.buf, n);
        self.buf.iter_mut().for_each(|b| drop(b.drain(..n)));
        self.frames += 1;
        self.total += n as u64;
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<W> {
        let n = self.buf[0].len();
        if n > 0 {
            self.emit(n)?;
        }
        let packed = (u64::from(RATE) << 44) | ((CHANNELS as u64 - 1) << 41) | (15 << 36) | self.total;
        self.out.seek(SeekFrom::Start(TOTAL_AT))?;
        self.out.write_all(&packed.to_be_bytes())?;
        self.out.write_all(&std::mem::replace(&mut self.md5, Md5::new()).digest())?;
        self.out.seek(SeekFrom::End(0))?;
        self.out.flush()?;
        Ok(self.out)
    }
}

fn bad(why: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.into())
}

enum Stop {
    Short,
    Crc,
    Bad(io::Error),
}

impl From<io::Error> for Stop {
    fn from(e: io::Error) -> Self {
        Stop::Bad(e)
    }
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn get(&mut self, bits: u32) -> Result<u64, Stop> {
        if self.pos + bits as usize > self.b.len() * 8 {
            return Err(Stop::Short);
        }
        let mut v = 0u64;
        let mut left = bits;
        while left > 0 {
            let byte = self.b[self.pos / 8];
            let off = (self.pos % 8) as u32;
            let take = (8 - off).min(left);
            let chunk = (u64::from(byte) >> (8 - off - take)) & ((1 << take) - 1);
            v = (v << take) | chunk;
            self.pos += take as usize;
            left -= take;
        }
        Ok(v)
    }

    fn signed(&mut self, bits: u32) -> Result<i64, Stop> {
        let v = self.get(bits)?;
        Ok(if bits == 0 { 0 } else { ((v << (64 - bits)) as i64) >> (64 - bits) })
    }

    fn unary(&mut self) -> Result<u64, Stop> {
        let mut q = 0;
        while self.get(1)? == 0 {
            q += 1;
        }
        Ok(q)
    }

    fn byte_pos(&self) -> usize {
        self.pos / 8
    }
}

fn read_utf8(r: &mut Reader) -> Result<u64, Stop> {
    let first = r.get(8)?;
    let n = (first as u8).leading_ones();
    match n {
        0 => return Ok(first),
        1 | 8 => return Err(bad("bad frame number").into()),
        _ => {}
    }
    let mut v = first & (0x7F >> n);
    for _ in 1..n {
        let c = r.get(8)?;
        if c & 0xC0 != 0x80 {
            return Err(bad("bad frame number").into());
        }
        v = (v << 6) | (c & 0x3F);
    }
    Ok(v)
}

fn read_subframe(r: &mut Reader, n: usize) -> Result<Vec<i16>, Stop> {
    if r.get(1)? != 0 {
        return Err(bad("bad subframe header").into());
    }
    let kind = r.get(6)?;
    let wasted = if r.get(1)? == 1 { r.unary()? as u32 + 1 } else { 0 };
    if wasted >= 16 {
        return Err(bad("bad wasted bits").into());
    }
    let bps = 16 - wasted;
    let mut s: Vec<i64> = Vec::with_capacity(n);
    match kind {
        0 => s.resize(n, r.signed(bps)?),
        1 => {
            for _ in 0..n {
                s.push(r.signed(bps)?);
            }
        }
        8..=12 => {
            let order = (kind - 8) as usize;
            if order > n {
                return Err(bad("predictor order exceeds block").into());
            }
            for _ in 0..order {
                s.push(r.signed(bps)?);
            }
            let res = read_residual(r, n, order)?;
            s.resize(n, 0);
            for (i, e) in (order..n).zip(res) {
                s[i] = predict(&s, i, order) + e;
            }
        }
        32..=63 => return Err(bad("LPC subframe: only xara-written FLAC is supported").into()),
        k => return Err(bad(format!("subframe type {k}: only xara-written FLAC is supported")).into()),
    }
    s.into_iter().map(|v| i16::try_from(v << wasted).map_err(|_| bad("sample out of range").into())).collect()
}

fn read_residual(r: &mut Reader, n: usize, warm: usize) -> Result<Vec<i64>, Stop> {
    if r.get(2)? != 0 {
        return Err(bad("residual coding method: only xara-written FLAC is supported").into());
    }
    let order = r.get(4)? as u32;
    if n % (1 << order) != 0 || (order > 0 && n >> order <= warm) {
        return Err(bad("bad partition order").into());
    }
    let size = n >> order;
    let mut out = Vec::with_capacity(n - warm);
    for j in 0..1usize << order {
        let count = if j == 0 { size - warm } else { size };
        let k = r.get(4)? as u32;
        if k == u32::from(ESCAPE) {
            let wid = r.get(5)? as u32;
            for _ in 0..count {
                out.push(r.signed(wid)?);
            }
        } else {
            for _ in 0..count {
                let q = r.unary()?;
                if q > u64::from(u32::MAX) >> k {
                    return Err(bad("residual out of range").into());
                }
                let u = (q << k) | r.get(k)?;
                out.push(((u >> 1) as i64) ^ -((u & 1) as i64));
            }
        }
    }
    Ok(out)
}

fn read_frame(b: &[u8], number: u64, out: &mut [Vec<i16>; CHANNELS]) -> Result<(usize, usize), Stop> {
    let mut r = Reader { b, pos: 0 };
    if r.get(16)? != 0xFFF8 {
        return Err(bad("bad frame sync").into());
    }
    let code = r.get(4)?;
    if r.get(4)? != 0xA || r.get(8)? != 0x28 {
        return Err(bad("frame is not 48 kHz, 3 channels, 16 bits").into());
    }
    if read_utf8(&mut r)? != number {
        return Err(bad(format!("frame {number} out of sequence")).into());
    }
    let n = match code {
        0xC => BLOCKSIZE,
        0x7 => r.get(16)? as usize + 1,
        _ => return Err(bad("bad block size code").into()),
    };
    let head = r.byte_pos();
    if r.get(8)? != u64::from(crc8(&b[..head])) {
        return Err(Stop::Crc);
    }
    let mut chans: Vec<Vec<i16>> = Vec::with_capacity(CHANNELS);
    for _ in 0..CHANNELS {
        chans.push(read_subframe(&mut r, n)?);
    }
    r.pos = r.pos.div_ceil(8) * 8;
    let end = r.byte_pos();
    if r.get(16)? != u64::from(crc16(&b[..end])) {
        return Err(Stop::Crc);
    }
    out.iter_mut().zip(chans).for_each(|(o, c)| o.extend(c));
    Ok((r.byte_pos(), n))
}

fn le32(c: &[u8], i: usize) -> io::Result<usize> {
    let b = c.get(i..i + 4).ok_or_else(|| bad("short vorbis comment"))?;
    Ok(u32::from_le_bytes(b.try_into().unwrap()) as usize)
}

fn string(c: &[u8], i: &mut usize) -> io::Result<String> {
    let n = le32(c, *i)?;
    let t = c.get(*i + 4..).and_then(|r| r.get(..n)).ok_or_else(|| bad("short vorbis comment"))?;
    *i += 4 + n;
    Ok(String::from_utf8_lossy(t).into_owned())
}

fn tags(c: &[u8]) -> io::Result<(String, Vec<String>)> {
    let mut i = 0;
    let vendor = string(c, &mut i)?;
    let count = le32(c, i)?;
    i += 4;
    let list = (0..count).map(|_| string(c, &mut i)).collect::<io::Result<_>>()?;
    Ok((vendor, list))
}

pub fn parse(b: &[u8]) -> io::Result<Capture> {
    if b.len() < 4 || &b[..4] != b"fLaC" {
        return Err(bad("not a FLAC file"));
    }
    let mut i = 4;
    let (mut info, mut comment) = (None, None);
    loop {
        let h = b.get(i..i + 4).ok_or_else(|| bad("truncated FLAC metadata"))?;
        let (last, kind) = (h[0] & 0x80 != 0, h[0] & 0x7F);
        let n = u32::from_be_bytes([0, h[1], h[2], h[3]]) as usize;
        let body = b.get(i + 4..i + 4 + n).ok_or_else(|| bad("truncated FLAC metadata"))?;
        match kind {
            0 if info.is_none() && n == 34 => info = Some(body),
            0 => return Err(bad("bad STREAMINFO")),
            4 => comment = Some(tags(body)?),
            127 => return Err(bad("bad metadata block")),
            _ => {}
        }
        if info.is_none() {
            return Err(bad("STREAMINFO is not first"));
        }
        i += 4 + n;
        if last {
            break;
        }
    }
    let info = info.unwrap();
    let not_ours = || bad("not a xara capture");
    let (vendor, tags) = comment.ok_or_else(not_ours)?;
    if !vendor.starts_with("xara ") || !tags.iter().any(|t| t == MARK) {
        return Err(not_ours());
    }
    let packed = u64::from_be_bytes(info[10..18].try_into().unwrap());
    let (rate, ch, bps) = ((packed >> 44) as u32, ((packed >> 41) & 7) + 1, ((packed >> 36) & 31) + 1);
    if (rate, ch, bps) != (RATE, CHANNELS as u64, 16) {
        return Err(bad(format!(
            "{rate} Hz, {ch} channels, {bps} bits: a xara capture is 48000 Hz, 3 channels, 16 bits"
        )));
    }
    let bs = (BLOCKSIZE as u16).to_be_bytes();
    if info[..4] != [bs[0], bs[1], bs[0], bs[1]] {
        return Err(bad("block size is not 4096"));
    }
    let total = packed & ((1 << 36) - 1);
    let mut channels: [Vec<i16>; CHANNELS] = Default::default();
    let mut number = 0;
    let mut decoded = 0u64;
    let mut short = false;
    while i < b.len() {
        if short {
            return Err(bad("short frame before the end"));
        }
        match read_frame(&b[i..], number, &mut channels) {
            Ok((len, n)) => {
                i += len;
                number += 1;
                decoded += n as u64;
                short = n != BLOCKSIZE;
            }
            Err(Stop::Short | Stop::Crc) if total == 0 => break,
            Err(Stop::Short) => return Err(bad("truncated FLAC frame")),
            Err(Stop::Crc) => return Err(bad(format!("frame {number}: CRC mismatch"))),
            Err(Stop::Bad(e)) => return Err(e),
        }
    }
    if total != 0 && decoded != total {
        return Err(bad(format!("{decoded} samples decoded, STREAMINFO says {total}")));
    }
    if total != 0 && info[18..34] != [0; 16] {
        let mut m = Md5::new();
        m.samples(&channels, channels[0].len());
        if m.digest() != info[18..34] {
            return Err(bad("MD5 mismatch"));
        }
    }
    Ok(Capture { rate, channels, tags })
}

pub fn read(path: &std::path::Path) -> io::Result<Capture> {
    parse(&std::fs::read(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn noise(n: usize, seed: u64) -> Vec<i16> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (s >> 48) as i16
            })
            .collect()
    }

    fn encode(ch: &[Vec<i16>; CHANNELS], chunk: usize) -> Vec<u8> {
        let mut w = Writer::new(Cursor::new(Vec::new()), &["START=1970-01-01T00:00:00Z".into()]).unwrap();
        let mut i = 0;
        while i < ch[0].len() {
            let j = (i + chunk).min(ch[0].len());
            w.write([&ch[0][i..j], &ch[1][i..j], &ch[2][i..j]]).unwrap();
            i = j;
        }
        w.finish().unwrap().into_inner()
    }

    fn round_trip(ch: [Vec<i16>; CHANNELS]) -> Vec<u8> {
        let b = encode(&ch, 480);
        let c = parse(&b).unwrap();
        assert_eq!(c.rate, RATE);
        assert!(c.channels == ch, "{} samples", ch[0].len());
        assert!(c.tags.iter().any(|t| t == "CHANNELS=rx,tx,ptt"));
        b
    }

    fn ptt(n: usize) -> Vec<i16> {
        (0..n).map(|i| ((i / 1000 % 8) as i16) << 8).collect()
    }

    #[test]
    fn crcs_match_the_reference_check_values() {
        assert_eq!(crc8(b"123456789"), 0xF4);
        assert_eq!(crc16(b"123456789"), 0xFEE8);
    }

    #[test]
    fn lengths_round_trip_bit_exact() {
        for n in [0, 1, 2, 3, 4, 5, 17, 480, 4095, 4096, 4097, 8192, 10_000] {
            round_trip([noise(n, 1), noise(n, 2), ptt(n)]);
        }
    }

    #[test]
    fn signals_round_trip_bit_exact() {
        let n = 3 * BLOCKSIZE + 123;
        let sine: Vec<i16> = (0..n).map(|i| (12000.0 * (i as f64 * 0.0731).sin()).round() as i16).collect();
        let steps: Vec<i16> = (0..n).map(|i| if i / 700 % 2 == 0 { 0 } else { 0x0700 }).collect();
        let full: Vec<i16> = (0..n).map(|i| if i % 2 == 0 { i16::MIN } else { i16::MAX }).collect();
        let rails: Vec<i16> = (0..n).map(|i| if i / 3 % 2 == 0 { i16::MIN } else { i16::MAX }).collect();
        round_trip([vec![0; n], vec![0; n], vec![0; n]]);
        round_trip([vec![i16::MIN; n], vec![i16::MAX; n], vec![-1; n]]);
        round_trip([full.clone(), rails.clone(), steps.clone()]);
        round_trip([sine.clone(), noise(n, 3), steps]);
        round_trip([noise(n, 4), sine, vec![0x0100; n]]);
        let ramp: Vec<i16> = (0..n).map(|i| (i as i32 * 977 % 65536 - 32768) as i16).collect();
        round_trip([ramp, rails, full]);
    }

    #[test]
    fn many_blocks_round_trip_and_compress() {
        let n = 40 * BLOCKSIZE + 7;
        let b = round_trip([noise(n, 5), vec![0; n], ptt(n)]);
        assert!(b.len() < n * 2 + n / 4, "{} bytes", b.len());
        let quiet: Vec<i16> = noise(n, 6).iter().map(|v| v >> 10).collect();
        let b = round_trip([quiet, vec![0; n], vec![0; n]]);
        assert!(b.len() < n, "{} bytes", b.len());
    }

    #[test]
    fn total_samples_are_patched() {
        let b = encode(&[vec![1; 5000], vec![2; 5000], vec![0; 5000]], 480);
        let packed = u64::from_be_bytes(b[18..26].try_into().unwrap());
        assert_eq!(packed & ((1 << 36) - 1), 5000);
        let mut m = Md5::new();
        m.update(&[1i16, 2, 0].repeat(5000).iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
        assert_eq!(&b[26..42], &m.digest());
    }

    #[test]
    fn md5_matches_the_reference_vectors() {
        let hex = |s: &[u8]| {
            let mut m = Md5::new();
            s.chunks(7).for_each(|c| m.update(c));
            m.digest().iter().map(|b| format!("{b:02x}")).collect::<String>()
        };
        assert_eq!(hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        let long = b"12345678901234567890123456789012345678901234567890123456789012345678901234567890";
        assert_eq!(hex(long), "57edf4a22be3c955ac49da2e2107b67a");
    }

    #[test]
    fn md5_mismatch_is_rejected() {
        let mut b = encode(&[noise(5000, 9), vec![0; 5000], vec![0; 5000]], 480);
        b[30] ^= 1;
        assert!(parse(&b).unwrap_err().to_string().contains("MD5 mismatch"));
        b[26..42].fill(0);
        parse(&b).unwrap();
    }

    #[test]
    fn truncation_decodes_a_prefix() {
        let n = 5 * BLOCKSIZE;
        let ch = [noise(n, 7), noise(n, 8), ptt(n)];
        let mut b = encode(&ch, 480);
        b[18..26].copy_from_slice(&(u64::from(RATE) << 44 | 2 << 41 | 15 << 36).to_be_bytes());
        b[26..42].fill(0);
        let start = b.windows(2).position(|w| w == [0xFF, 0xF8]).unwrap();
        let mut seen = Vec::new();
        for cut in (start..=b.len()).step_by(997).chain([start, start + 1, b.len() - 1, b.len()]) {
            let c = parse(&b[..cut]).unwrap();
            let got = c.channels[0].len();
            assert_eq!(got % BLOCKSIZE, 0);
            assert!(c.channels.iter().zip(&ch).all(|(c, o)| c[..] == o[..got]));
            seen.push(got);
        }
        assert!(seen.contains(&0) && seen.contains(&n) && seen.contains(&(2 * BLOCKSIZE)));
        assert!(parse(&b[..start - 3]).is_err());
        let full = encode(&ch, 480);
        assert!(parse(&full[..full.len() - 1]).unwrap_err().to_string().contains("truncated"));
    }

    #[test]
    fn corruption_is_a_crc_error_when_complete() {
        let n = 2 * BLOCKSIZE;
        let mut b = encode(&[noise(n, 9), vec![0; n], vec![0; n]], 480);
        let at = b.len() - 100;
        b[at] ^= 0x10;
        assert!(parse(&b).unwrap_err().to_string().contains("CRC"));
    }

    fn err(b: &[u8]) -> String {
        parse(b).expect_err("a foreign file must be rejected").to_string()
    }

    fn with_vendor(vendor: &str, tags: &[&str]) -> Vec<u8> {
        let mut b = encode(&[vec![0; 10], vec![0; 10], vec![0; 10]], 480);
        let len = u32::from_be_bytes([0, b[43], b[44], b[45]]) as usize;
        let mut c = Vec::new();
        put_string(&mut c, vendor);
        c.extend((tags.len() as u32).to_le_bytes());
        tags.iter().for_each(|t| put_string(&mut c, t));
        b[43..46].copy_from_slice(&(c.len() as u32).to_be_bytes()[1..]);
        b.splice(46..46 + len, c);
        b
    }

    #[test]
    fn foreign_files_are_rejected() {
        assert!(parse(&with_vendor("xara 9.9.9", &[MARK])).is_ok());
        assert!(err(&with_vendor("reference libFLAC 1.5.0 20250211", &[MARK])).contains("not a xara capture"));
        assert!(err(&with_vendor("xara 9.9.9", &["XARA_CAPTURE=0"])).contains("not a xara capture"));
        assert!(err(b"RIFF....WAVE").contains("not a FLAC"));
        let mut b = encode(&[vec![0; 10], vec![0; 10], vec![0; 10]], 480);
        b[43] = 0x7F;
        assert!(err(&b).contains("truncated FLAC metadata"));
        let mut b = encode(&[vec![0; 10], vec![0; 10], vec![0; 10]], 480);
        b[18..26].copy_from_slice(&(44100u64 << 44 | 1 << 41 | 15 << 36 | 10).to_be_bytes());
        assert!(err(&b).contains("44100 Hz, 2 channels"));
    }

    fn lpc_frame() -> Vec<u8> {
        let mut w = Bits::default();
        w.put(0xFFF8, 16);
        w.put(0x7A, 8);
        w.put(0x28, 8);
        w.put(0, 8);
        w.put(9, 16);
        let c = crc8(&w.out);
        w.put(u64::from(c), 8);
        w.put(0x40, 8);
        w.align();
        let c = crc16(&w.out);
        w.put(u64::from(c), 16);
        w.out
    }

    #[test]
    fn foreign_subframes_are_rejected() {
        let mut b = encode(&[vec![], vec![], vec![]], 480);
        b[18..26].copy_from_slice(&(u64::from(RATE) << 44 | 2 << 41 | 15 << 36 | 10).to_be_bytes());
        b.extend(lpc_frame());
        assert!(err(&b).contains("LPC subframe: only xara-written FLAC is supported"));
        let mut b = encode(&[vec![1, 2, 3], vec![0; 3], vec![0; 3]], 480);
        let at = b.windows(2).position(|w| w == [0xFF, 0xF8]).unwrap();
        b[at + 2] = 0x79;
        assert!(err(&b).contains("frame is not 48 kHz"));
    }

    #[test]
    fn silent_partitions_use_a_zero_width_escape() {
        let res: Vec<i64> =
            (0..BLOCKSIZE).map(|i| if i < 2048 { 0 } else { i64::from(noise(1, i as u64)[0] >> 6) }).collect();
        let p = plan(&res, BLOCKSIZE, 0);
        assert!(p.order > 0 && p.params[0] == (ESCAPE, 0) && p.params.last().unwrap().1 > 0);
    }

    #[test]
    fn frame_numbers_use_extended_utf8() {
        for v in [0u64, 0x7F, 0x80, 0x7FF, 0x800, 0xFFFF, 0x1_0000, 0x7FF_FFFF, 0x7FFF_FFFF, 0xF_FFFF_FFFF] {
            let mut w = Bits::default();
            utf8(&mut w, v);
            assert_eq!(read_utf8(&mut Reader { b: &w.out, pos: 0 }).ok(), Some(v));
        }
    }
}
