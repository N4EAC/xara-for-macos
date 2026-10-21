#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};
use xara_testkit::json::Value;
use xara_ui::ws;

pub fn open_raw(addr: SocketAddr, head: &str) -> TcpStream {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(s, "{head}\r\n\r\n").unwrap();
    s
}

pub fn open(addr: SocketAddr, head: &str) -> TcpStream {
    if head.contains("\r\nHost:") {
        return open_raw(addr, head);
    }
    let (first, rest) = head.split_once("\r\n").map_or((head, String::new()), |(f, r)| (f, format!("\r\n{r}")));
    open_raw(addr, &format!("{first}\r\nHost: {addr}{rest}"))
}

pub fn head_of(s: &mut TcpStream) -> String {
    let mut h = Vec::new();
    let mut b = [0u8; 1];
    while !h.ends_with(b"\r\n\r\n") {
        s.read_exact(&mut b).unwrap();
        h.push(b[0]);
    }
    String::from_utf8(h).unwrap()
}

pub fn upgrade(addr: SocketAddr, extra: &str) -> (TcpStream, String) {
    let mut s = open(
        addr,
        &format!(
            "GET /ws HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13{extra}"
        ),
    );
    let h = head_of(&mut s);
    (s, h)
}

pub fn server_frame(s: &mut TcpStream) -> (u8, Vec<u8>) {
    let mut h = [0u8; 2];
    s.read_exact(&mut h).unwrap();
    assert_eq!(h[1] & 0x80, 0, "server frames are unmasked");
    let n = match h[1] {
        126 => {
            let mut b = [0u8; 2];
            s.read_exact(&mut b).unwrap();
            u64::from(u16::from_be_bytes(b))
        }
        127 => {
            let mut b = [0u8; 8];
            s.read_exact(&mut b).unwrap();
            u64::from_be_bytes(b)
        }
        n => u64::from(n),
    };
    let mut p = vec![0u8; n as usize];
    s.read_exact(&mut p).unwrap();
    (h[0] & 0x0F, p)
}

pub fn message(s: &mut TcpStream) -> Value {
    loop {
        let (op, p) = server_frame(s);
        if op != ws::PING {
            assert_eq!(op, ws::TEXT);
            return xara_testkit::json::from_slice(&p).unwrap();
        }
    }
}

pub fn until(s: &mut TcpStream, f: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..200 {
        let m = message(s);
        if f(&m) {
            return m;
        }
    }
    panic!("message not seen");
}

pub fn masked(op: u8, p: &[u8]) -> Vec<u8> {
    let mask = [9, 8, 7, 6];
    let mut f = ws::frame(op, p);
    f.truncate(f.len() - p.len());
    f[1] |= 0x80;
    f.extend(mask);
    f.extend(p.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    f
}

pub fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !f() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub fn replayed(addr: SocketAddr, n: u64) -> (TcpStream, Value) {
    let mut out = None;
    eventually("history replay", || {
        let (mut s, h) = upgrade(addr, "");
        assert!(h.starts_with("HTTP/1.1 101"), "{h}");
        let hello = message(&mut s);
        let done = hello["replay"].as_u64() == Some(n);
        out = Some((s, hello));
        done
    });
    out.unwrap()
}
