use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use xara_testkit::json::Value;
use xara_ui::bus::{Publisher, channel};
use xara_ui::messages::Info;
use xara_ui::server::{Pages, UiServer};
use xara_ui::ws;

mod common {
    pub mod ws;
}
use common::ws::{eventually, masked, message, open, replayed, server_frame, until};

const INFO: Info = Info { version: "test", mode: "hf", span_hz: 3000 };
const PAGES: Pages = Pages {
    name: "xara-monitor",
    index: "<title>monitor</title><script src=\"app.js\"></script>",
    app: "// monitor app",
    style: "body{}",
};

fn start() -> (Publisher, SocketAddr) {
    let (p, sub) = channel(64);
    let ui = UiServer::bind_readonly("127.0.0.1", 0, sub, INFO, PAGES, Vec::new()).unwrap();
    (p, ui.addr)
}

fn get(addr: SocketAddr, head: &str) -> String {
    let mut out = String::new();
    open(addr, head).read_to_string(&mut out).unwrap();
    out
}

fn upgrade(addr: SocketAddr, extra: &str) -> TcpStream {
    let (s, h) = common::ws::upgrade(addr, extra);
    assert!(h.starts_with("HTTP/1.1 101"), "{h}");
    s
}

#[test]
fn serves_its_own_pages_events_and_status() {
    let (p, addr) = start();
    assert!(get(addr, "GET / HTTP/1.1").contains("<title>monitor</title>"));
    assert!(get(addr, "GET /app.js HTTP/1.1").ends_with("// monitor app"));
    assert!(get(addr, "GET /style.css HTTP/1.1").ends_with("body{}"));
    for (path, kind) in [("/common.js", "text/javascript"), ("/common.css", "text/css")] {
        let r = get(addr, &format!("GET {path} HTTP/1.1"));
        assert!(r.starts_with("HTTP/1.1 200") && r.contains(&format!("Content-Type: {kind}")), "{path}: {r}");
    }
    let bad = get(addr, "GET / HTTP/1.1\r\nHost: evil.example");
    assert!(bad.starts_with("HTTP/1.1 403") && bad.contains("start xara-monitor with --ui-host"), "{bad}");
    p.event(r#"{"t":"ctrl","ts":1}"#.into());
    let (mut ws, hello) = replayed(addr, 1);
    assert_eq!(
        (hello["t"].as_str(), hello["control"].as_bool(), hello["locked"].as_bool()),
        (Some("hello"), Some(false), Some(false))
    );
    assert_eq!((hello["replay"].as_u64(), message(&mut ws)["t"].as_str()), (Some(1), Some("ctrl")));
    p.status(r#"{"t":"status","lag":0.5}"#.into());
    assert_eq!(until(&mut ws, |m| m["t"] == "status")["lag"], 0.5);
    let mut stream = BufReader::new(open(addr, "GET /stream HTTP/1.1"));
    let mut line = String::new();
    while line != "\r\n" {
        line.clear();
        stream.read_line(&mut line).unwrap();
    }
    p.event(r#"{"t":"learned","ts":2}"#.into());
    let mut seen = Vec::new();
    while !seen.iter().any(|v: &Value| v["t"] == "learned") {
        line.clear();
        stream.read_line(&mut line).unwrap();
        seen.push(xara_testkit::json::from_str(&line).unwrap());
    }
}

#[test]
fn every_control_and_setting_is_refused_whatever_the_page_sends() {
    let (_p, addr) = start();
    let login = get(addr, "GET /?token=0123456789abcdef HTTP/1.1");
    assert!(login.starts_with("HTTP/1.1 403") && !login.contains("Set-Cookie"), "{login}");
    let guess = format!("\r\nCookie: xara_token_{}=0123456789abcdef", addr.port());
    let mut ws = upgrade(addr, &guess);
    assert_eq!(message(&mut ws)["control"], false);
    for cmd in ["TUNE ON", "TUNE OFF", "ABORT", "RESTART", "DRIVE -3"] {
        ws.write_all(&masked(ws::TEXT, cmd.as_bytes())).unwrap();
        let r = until(&mut ws, |m| m["t"] == "control");
        assert_eq!(
            (r["ok"].as_bool(), r["what"].as_str()),
            (Some(false), Some(cmd.to_ascii_lowercase().as_str())),
            "{cmd}: {r}"
        );
        assert!(r["reason"].as_str().unwrap().contains("not enabled"), "{cmd}: {r}");
    }
    for cmd in ["SETTINGS ?", "DEVICES ?", "SETTINGS SAVE\ncall N0CALL", "SETTINGS RESET"] {
        ws.write_all(&masked(ws::TEXT, cmd.as_bytes())).unwrap();
        let r = until(&mut ws, |m| m["t"] == "settings_result");
        assert_eq!(r["ok"], false, "{cmd}: {r}");
    }
}

#[test]
fn waterfall_rows_come_from_the_bus_audio() {
    let (p, addr) = start();
    let mut ws = upgrade(addr, "");
    assert_eq!(message(&mut ws)["t"], "hello");
    assert!(!p.wants_audio());
    ws.write_all(&masked(ws::TEXT, b"WATERFALL ON")).unwrap();
    assert_eq!(until(&mut ws, |m| m["t"] == "waterfall")["span"], 3000.0);
    eventually("audio request", || p.wants_audio());
    let tone: Vec<f64> = (0..48_000).map(|i| (i as f64 * 0.2).sin() * 0.5).collect();
    for c in tone.chunks(480) {
        p.audio(c);
    }
    loop {
        let (op, b) = server_frame(&mut ws);
        if op == ws::BINARY {
            assert_eq!(b[0], 1);
            break;
        }
    }
}
