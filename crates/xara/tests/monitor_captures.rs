mod common;

use common::{Seen, captures_dir};
use xara::monitor::{Monitor, MonitorOpts};
use xara_phy::hf::ctrl::Role;
use xara_phy::hf::level::Bandwidth;

fn mixed(name: &str, secs: f64) -> Vec<f64> {
    let [a, b] = ["A", "B"].map(|s| xara::wav::read(&captures_dir().join(format!("{name}_{s}.wav"))).unwrap());
    assert_eq!((a.rate, b.rate), (48000, 48000));
    let n = a.samples.len().max(b.samples.len()).min((secs * 48000.0) as usize);
    (0..n).map(|i| a.samples.get(i).unwrap_or(&0.0) + b.samples.get(i).unwrap_or(&0.0)).collect()
}

fn monitor(name: &str, bw: Bandwidth, secs: f64) -> (Monitor, Seen, String) {
    let mut m = Monitor::new(bw, &MonitorOpts::default());
    let mut seen = Seen::default();
    let mut log = Vec::new();
    for c in mixed(name, secs).chunks(480) {
        m.feed(c);
    }
    m.finish();
    while let Some(l) = m.pop() {
        log.push(format!("{:9.3} {}", l.t, l.text));
        seen.add(&l.rec);
    }
    (m, seen, log.join("\n"))
}

fn follows_session(name: &str, bw: Bandwidth, secs: f64, bytes: usize) {
    let (m, s, log) = monitor(name, bw, secs);
    eprintln!("{name}: {}", log.lines().last().unwrap());
    assert!(log.contains("learned answerer key #D8A3D7"), "{name}:\n{log}");
    assert_eq!(s.callers, ["N0CALL"], "{name}:\n{log}");
    assert!(s.has(Some(Role::Caller), &[26, 31, 36]), "{name}:\n{log}");
    assert_eq!(m.stats.frames_bad, 0, "{name}:\n{log}");
    assert_eq!((s.delivered[0].len(), s.delivered[1].len(), s.delivered[2].len()), (bytes, 0, 0), "{name}:\n{log}");
}

#[test]
#[ignore = "needs the VARA lab captures in $XARA_CAPTURES; run with --release --ignored"]
fn monitor_follows_vara_session_500() {
    follows_session("session_500", Bandwidth::Hz500, 62.1, 430);
}

#[test]
#[ignore = "needs the VARA lab captures in $XARA_CAPTURES; run with --release --ignored"]
fn monitor_follows_vara_session_2300() {
    follows_session("session_2300", Bandwidth::Hz2300, 62.7, 890);
}

#[test]
#[ignore = "needs the VARA lab captures in $XARA_CAPTURES; run with --release --ignored"]
fn monitor_follows_vara_session_2750() {
    follows_session("session_2750", Bandwidth::Hz2750, 64.3, 890);
}

#[test]
#[ignore = "needs the VARA lab captures in $XARA_CAPTURES; run with --release --ignored"]
fn monitor_reads_vara_text_session() {
    let (_, s, log) = monitor("text_2300_chatoff", Bandwidth::Hz2300, f64::INFINITY);
    let d = &s.delivered[0];
    assert_eq!(d.len(), 145, "{log}");
    assert!(d.starts_with(b"HELLO N1CALL DE N0CALL. THIS IS A VARA DATA PORT DELIVERY TE"), "{log}");
    assert!(d.ends_with(b"HE QUICK BROWN FOX JUMPS OVER THE LAZY DOG 0123456789.\r\n"), "{log}");
    assert!(s.has(None, &[3]), "no QRT:\n{log}");
}
