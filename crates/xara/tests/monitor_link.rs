mod common;

use common::Seen;
use common::air::{self, BLOCK};
use xara::monitor::{Monitor, MonitorOpts};
use xara::station::hf::Station;
use xara::station::{Modem, Options, Report};
use xara_arq::compression::Compression;
use xara_arq::hf::session::Config;
use xara_arq::host::HostEvent;
use xara_phy::call::call_hash;
use xara_phy::hf::ctrl::Role;
use xara_phy::hf::level::Bandwidth;

const CALLER: &str = "N0CALL";
const ANSWERER: &str = "N1CALL";

struct Air {
    link: air::Air<Station, 2>,
    mon: Monitor,
    lines: Vec<String>,
    json: Vec<String>,
    seen: Seen,
}

impl Air {
    fn new(bw: Bandwidth, comp: Compression, noise: f64) -> Self {
        let cfg = |call: &str, seed| {
            let mut c = Config { seed, ..Config::new(bw, call) };
            (c.host.registered, c.host.compression) = (true, comp);
            c
        };
        let st = [Station::new(cfg(CALLER, 3), Options::default()), Station::new(cfg(ANSWERER, 5), Options::default())];
        Self {
            link: air::Air::pair(st, [2400; 2], noise, |r| !matches!(r, Report::Host(HostEvent::Connected { .. }))),
            mon: Monitor::new(bw, &MonitorOpts::default()),
            lines: Vec::new(),
            json: Vec::new(),
            seen: Seen::default(),
        }
    }

    fn linked(&self) -> bool {
        self.link.events.iter().all(|e| e.iter().any(|(_, r)| matches!(r, Report::Host(HostEvent::Connected { .. }))))
    }

    fn step(&mut self) {
        self.link.step();
        let (w, rng, noise) = (&self.link.wire, &mut self.link.rng, self.link.noise);
        let tail = |i: usize| w[i].range(w[i].len() - BLOCK..);
        let heard: Vec<f64> = tail(0).zip(tail(1)).map(|(a, b)| a + b + noise * rng.gauss()).collect();
        self.mon.feed(&heard);
        self.drain();
    }

    fn drain(&mut self) {
        while let Some(l) = self.mon.pop() {
            self.lines.push(format!("{:9.3} {}", l.t, l.text));
            self.json.push(l.rec.json(l.t, None, &l.text));
            self.seen.add(&l.rec);
        }
    }

    fn run_until(&mut self, secs: f64, mut done: impl FnMut(&Self) -> bool) -> bool {
        let limit = self.link.secs() + secs;
        while self.link.secs() < limit {
            if done(self) {
                return true;
            }
            self.step();
        }
        done(self)
    }

    fn log(&self) -> String {
        self.lines.join("\n")
    }
}

fn text(n: usize, who: &str) -> Vec<u8> {
    let line =
        format!("Subject: net report\r\nThe weather station at {who} reports clear skies; check-in tonight 73\r\n");
    line.bytes().cycle().take(n).collect()
}

fn session(bw: Bandwidth, comp: Compression, noise: f64, pa: Vec<u8>, pb: Vec<u8>) -> Air {
    run_session(Air::new(bw, comp, noise), &pa, &pb, 15.0)
}

fn run_session(mut air: Air, pa: &[u8], pb: &[u8], tail: f64) -> Air {
    air.link.st[0].connect(CALLER, ANSWERER, &[]);
    assert!(air.run_until(40.0, |a| a.linked()), "no link:\n{}", air.log());
    air.link.st[0].send(pa);
    air.link.st[1].send(pb);
    let secs = 30.0 + (pa.len() + pb.len()) as f64 / 8.0;
    let done = air.run_until(secs, |a| a.link.got[1].len() >= pa.len() && a.link.got[0].len() >= pb.len());
    assert!(done, "transfer stalled:\n{}", air.log());
    air.link.st[0].disconnect();
    assert!(air.run_until(40.0, |a| a.link.st.iter().all(Station::idle)), "no disconnect:\n{}", air.log());
    air.run_until(tail, |_| false);
    air.mon.finish();
    air.drain();
    air
}

fn top_level_keys(j: &str) -> usize {
    let (mut depth, mut in_str, mut esc, mut keys) = (0, false, false, 0);
    for c in j.chars() {
        match (in_str, esc, c) {
            (true, true, _) => esc = false,
            (true, false, '\\') => esc = true,
            (true, false, '"') => in_str = false,
            (true, ..) => {}
            (false, _, '"') => in_str = true,
            (false, _, '{' | '[') => depth += 1,
            (false, _, '}' | ']') => depth -= 1,
            (false, _, ':') if depth == 1 => keys += 1,
            _ => {}
        }
    }
    keys
}

fn check_json(air: &Air, pa: &[u8], pb: &[u8]) {
    let mut streams = [Vec::new(), Vec::new()];
    assert_eq!(air.json.len(), air.lines.len());
    for (j, line) in air.json.iter().zip(&air.lines) {
        let v: xara_testkit::json::Value = xara_testkit::json::from_str(j).unwrap_or_else(|e| panic!("{e}: {j}"));
        let o = v.as_object().unwrap();
        assert_eq!(o.len(), top_level_keys(j), "duplicate key in {j}");
        assert_eq!(o["text"].as_str(), Some(&line[10..]), "{j}");
        if o["t"] == "payload" && o["status"] == "data" {
            let i = match o["sender"].as_str() {
                Some("caller") => 0,
                Some("answerer") => 1,
                _ => continue,
            };
            streams[i].extend(o["data"].as_str().unwrap().chars().map(|c| u8::try_from(u32::from(c)).unwrap()));
        }
    }
    assert_eq!(streams, [pa, pb], "payload records do not reassemble the streams");
}

fn check(air: &Air, pa: &[u8], pb: &[u8]) {
    check_json(air, pa, pb);
    let log = air.log();
    let s = &air.seen;
    let answerer = format!("#{:06X}", call_hash(ANSWERER.as_bytes()));
    assert!(log.contains(&format!("learned answerer key {answerer}")), "{log}");
    assert_eq!(s.callers, [CALLER], "{log}");
    let has = |sender: Role, codes: &[u32]| s.has(Some(sender), codes);
    assert!(has(Role::Caller, &[26, 31, 36]), "no connect request:\n{log}");
    assert!(has(Role::Answerer, &[15, 16, 17, 18, 22, 23, 24, 25, 41, 42, 43, 44]), "no connect reply:\n{log}");
    assert!(has(Role::Caller, &[3, 46]) || has(Role::Answerer, &[3, 46]), "no QRT:\n{log}");
    assert_eq!(s.delivered[0], pa, "caller → answerer stream differs:\n{log}");
    assert_eq!(s.delivered[1], pb, "answerer → caller stream differs:\n{log}");
    assert!(s.delivered[2].is_empty(), "unattributed data:\n{log}");
}

#[test]
fn monitor_follows_unknown_text_session() {
    let (pa, pb) = (text(6000, CALLER), text(1500, ANSWERER));
    let air = session(Bandwidth::Hz2300, Compression::Text, 0.002, pa.clone(), pb.clone());
    check(&air, &pa, &pb);
    assert!(air.mon.stats.levels.keys().any(|&l| l > 4), "link never left FSK:\n{}", air.log());
}

#[test]
fn monitor_follows_unknown_files_session_500() {
    let (pa, pb) = (text(2500, CALLER), text(600, ANSWERER));
    let air = session(Bandwidth::Hz500, Compression::Files, 0.002, pa.clone(), pb.clone());
    check(&air, &pa, &pb);
}

#[test]
fn monitor_drops_idle_keys_after_the_link() {
    let (pa, pb) = (text(1500, CALLER), text(300, ANSWERER));
    let mut air = Air::new(Bandwidth::Hz2300, Compression::Text, 0.002);
    air.mon = Monitor::new(Bandwidth::Hz2300, &MonitorOpts { key_idle: 20.0, ..Default::default() });
    let air = run_session(air, &pa, &pb, 45.0);
    check(&air, &pa, &pb);
    let log = air.log();
    let ended = log.find("ended (QRT)").unwrap_or_else(|| panic!("no QRT link end:\n{log}"));
    for note in [
        format!("dropped answerer key #{:06X}", call_hash(ANSWERER.as_bytes())),
        format!("dropped caller key {CALLER}"),
    ] {
        let i = log.find(&note).unwrap_or_else(|| panic!("no \"{note}\":\n{log}"));
        assert!(i > ended, "{note} before the link ended:\n{log}");
    }
}
