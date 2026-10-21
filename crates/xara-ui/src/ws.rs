use std::io::{self, Read};

pub const TEXT: u8 = 0x1;
pub const BINARY: u8 = 0x2;
pub const CLOSE: u8 = 0x8;
pub const PING: u8 = 0x9;
pub const PONG: u8 = 0xA;
const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Frame {
    pub op: u8,
    pub payload: Vec<u8>,
}

pub(crate) fn accept_key(key: &str) -> String {
    base64(&sha1(format!("{}{GUID}", key.trim()).as_bytes()))
}

pub fn frame(op: u8, payload: &[u8]) -> Vec<u8> {
    let n = payload.len();
    let mut f = Vec::with_capacity(n + 10);
    f.push(0x80 | op);
    match n {
        0..=125 => f.push(n as u8),
        126..=0xFFFF => {
            f.push(126);
            f.extend_from_slice(&(n as u16).to_be_bytes());
        }
        _ => {
            f.push(127);
            f.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    f.extend_from_slice(payload);
    f
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Fail {
    Io,
    Close(u16),
}

impl From<io::Error> for Fail {
    fn from(_: io::Error) -> Self {
        Fail::Io
    }
}

const PROTOCOL: Fail = Fail::Close(1002);

fn read_frame(r: &mut impl Read, max: usize) -> Result<(bool, Frame), Fail> {
    let mut h = [0u8; 2];
    r.read_exact(&mut h)?;
    let (fin, op) = (h[0] & 0x80 != 0, h[0] & 0x0F);
    if h[0] & 0x70 != 0 || h[1] & 0x80 == 0 {
        return Err(PROTOCOL);
    }
    let n = match h[1] & 0x7F {
        126 => {
            let mut b = [0u8; 2];
            r.read_exact(&mut b)?;
            u64::from(u16::from_be_bytes(b))
        }
        127 => {
            let mut b = [0u8; 8];
            r.read_exact(&mut b)?;
            u64::from_be_bytes(b)
        }
        n => u64::from(n),
    };
    match op {
        0..=2 if n > max as u64 => return Err(Fail::Close(1009)),
        0..=2 => {}
        CLOSE | PING | PONG if fin && n <= 125 => {}
        _ => return Err(PROTOCOL),
    }
    let mut mask = [0u8; 4];
    r.read_exact(&mut mask)?;
    let mut payload = vec![0u8; n as usize];
    r.read_exact(&mut payload)?;
    for (i, b) in payload.iter_mut().enumerate() {
        *b ^= mask[i % 4];
    }
    Ok((fin, Frame { op, payload }))
}

#[derive(Default)]
pub(crate) struct Reader {
    op: u8,
    buf: Vec<u8>,
}

impl Reader {
    pub(crate) fn next(&mut self, r: &mut impl Read, max: usize) -> Result<Frame, Fail> {
        loop {
            let (fin, f) = read_frame(r, max - self.buf.len())?;
            match (f.op, self.op) {
                (CLOSE | PING | PONG, _) => return Ok(f),
                (0, 0) => return Err(PROTOCOL),
                (_, 0) if fin => return Ok(f),
                (0, _) => {}
                (_, 0) => self.op = f.op,
                _ => return Err(PROTOCOL),
            }
            self.buf.extend_from_slice(&f.payload);
            if fin {
                return Ok(Frame { op: std::mem::take(&mut self.op), payload: std::mem::take(&mut self.buf) });
            }
        }
    }
}

fn sha1(msg: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let mut m = msg.to_vec();
    m.push(0x80);
    while m.len() % 64 != 56 {
        m.push(0);
    }
    m.extend_from_slice(&((msg.len() as u64) * 8).to_be_bytes());
    for block in m.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, c) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes(c.try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..20 => ((b & c) | (!b & d), 0x5A82_7999),
                20..40 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..60 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wi);
            (e, d, c, b, a) = (d, c, b.rotate_left(30), a, t);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (o, x) in out.chunks_exact_mut(4).zip(h) {
        o.copy_from_slice(&x.to_be_bytes());
    }
    out
}

fn base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in b.chunks(3) {
        let n =
            (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            s.push(if i <= c.len() { A[(n >> (18 - 6 * i)) as usize & 63] as char } else { '=' });
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masked(op: u8, payload: &[u8]) -> Vec<u8> {
        let mut f = frame(op, payload);
        let at = f.len() - payload.len();
        f[1] |= 0x80;
        let mask = [0x37, 0xFA, 0x21, 0x3D];
        let body: Vec<u8> = payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]).collect();
        f.truncate(at);
        f.extend_from_slice(&mask);
        f.extend(body);
        f
    }

    #[test]
    fn rfc6455_accept_key() {
        assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn sha1_and_base64_vectors() {
        assert_eq!(base64(&sha1(b"")), "2jmj7l5rSw0yVb/vlWAYkK/YBwk=");
        assert_eq!(base64(&sha1(&[b'a'; 1000])), "KR6abGaZSUm1e6XmUDYemPw2sbo=");
        assert_eq!([base64(b"f"), base64(b"fo"), base64(b"foo")], ["Zg==", "Zm8=", "Zm9v"]);
    }

    #[test]
    fn frame_lengths() {
        assert_eq!(frame(TEXT, b"hi"), [0x81, 2, b'h', b'i']);
        assert_eq!(frame(BINARY, &[0; 300])[..4], [0x82, 126, 1, 44]);
        assert_eq!(frame(BINARY, &[0; 70000])[..10], [0x82, 127, 0, 0, 0, 0, 0, 1, 0x11, 0x70]);
    }

    fn read(bytes: &[u8]) -> Result<Frame, Fail> {
        Reader::default().next(&mut &bytes[..], 100)
    }

    fn part(op: u8, payload: &[u8], fin: bool) -> Vec<u8> {
        let mut f = masked(op, payload);
        if !fin {
            f[0] &= 0x7F;
        }
        f
    }

    #[test]
    fn client_frames() {
        for n in [0, 5, 200, 70000] {
            let p: Vec<u8> = (0..n).map(|i| i as u8).collect();
            let f = Reader::default().next(&mut &masked(TEXT, &p)[..], 1 << 20).unwrap();
            assert_eq!(f, Frame { op: TEXT, payload: p });
        }
        assert_eq!(read(&frame(TEXT, b"x")), Err(Fail::Close(1002)));
        assert_eq!(read(&masked(TEXT, &[0; 200])), Err(Fail::Close(1009)));
        assert_eq!(read(&masked(PING, b"x")[..3]), Err(Fail::Io));
    }

    #[test]
    fn fragmented_messages() {
        let mut b =
            [part(TEXT, b"he", false), part(PING, b"p", true), part(0, b"l", false), part(0, b"lo", true)].concat();
        b.extend(masked(BINARY, b"z"));
        let mut r = Reader::default();
        let mut s = &b[..];
        assert_eq!(r.next(&mut s, 100), Ok(Frame { op: PING, payload: b"p".to_vec() }));
        assert_eq!(r.next(&mut s, 100), Ok(Frame { op: TEXT, payload: b"hello".to_vec() }));
        assert_eq!(r.next(&mut s, 100), Ok(Frame { op: BINARY, payload: b"z".to_vec() }));
        let big = [part(TEXT, &[0; 60], false), part(0, &[0; 60], true)].concat();
        assert_eq!(read(&big), Err(Fail::Close(1009)));
    }

    #[test]
    fn protocol_errors() {
        for bad in [
            part(0, b"x", true),
            [part(TEXT, b"x", false), part(BINARY, b"y", true)].concat(),
            part(PING, b"x", false),
            masked(PING, &[0; 126]),
            masked(0x3, b"x"),
            masked(0xB, b""),
        ] {
            assert_eq!(read(&bad), Err(Fail::Close(1002)));
        }
        let mut rsv = masked(TEXT, b"x");
        rsv[0] |= 0x40;
        assert_eq!(read(&rsv), Err(Fail::Close(1002)));
    }
}
