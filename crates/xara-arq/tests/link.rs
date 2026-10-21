mod golden;
mod sim;

use sim::{Channel, Payload, Sess, check, connected, payload};
use xara_arq::compression::Compression;
use xara_arq::hf::session::{Config, CtrlIn, DataIn, FrameIn, Listen, Output, Session, State, Tx};
use xara_arq::host::HostEvent;
use xara_arq::time::{SECOND, Time};
use xara_phy::hf::ctrl::{Fmt, leadin_symbols};
use xara_phy::hf::level::Bandwidth;

type Link = sim::Link<Session>;

const CLEAN: Channel = Channel { snr: 25.0, frame_loss: 0.0, ctrl_loss: 0.0, miss: 0.0 };

impl Sess for Session {
    type Config = Config;
    type Out = Output;
    type Tx = Tx;
    type Listen = Listen;
    type State = State;
    const LATENCY: Time = SECOND / 10;

    fn open(c: Config) -> Self {
        Session::new(c)
    }

    fn next(&mut self) -> Option<Output> {
        self.pop()
    }

    fn split(o: Output) -> sim::Out<Self> {
        match o {
            Output::Listen(l) => sim::Out::Listen(l),
            Output::Host(h) => sim::Out::Host(h),
            Output::Tx(t) => sim::Out::Tx(t),
        }
    }

    fn golden_line(o: &Output) -> String {
        match o {
            Output::Tx(Tx::Data { level, frames }) => {
                format!(
                    "Tx(Data {{ level: {level}, frames: [{}] }})",
                    frames.iter().map(|f| golden::digest(f)).collect::<Vec<_>>().join(", ")
                )
            }
            Output::Host(HostEvent::Data(d)) => format!("Host(Data({}))", golden::digest(d)),
            o => format!("{o:?}"),
        }
    }

    fn deadline(&self) -> Option<Time> {
        self.next_deadline()
    }

    fn tick(&mut self, now: Time) {
        self.poll(now)
    }

    fn tx_done(&mut self, now: Time) {
        self.on_tx_done(now)
    }

    fn st(&self) -> State {
        self.state()
    }

    fn airtime(&self, t: &Tx) -> (Time, Time) {
        let s = match t {
            Tx::Ctrl { code, fmt, extra } => {
                f64::from(leadin_symbols(*fmt, *code, *extra) + fmt.params().nsym - 1) * 2048.0 / 48000.0
            }
            Tx::Data { level, frames } => {
                let l = self.bandwidth().level(*level);
                l.airtime() + (frames.len() - 1) as f64 * f64::from(l.data_symbols()) * l.symbol_period()
            }
            Tx::CwId { .. } => 3.0,
        };
        ((s * SECOND as f64) as Time, 0)
    }

    fn what(t: &Tx) -> String {
        match t {
            Tx::Ctrl { code, fmt, .. } => format!("c{code}f{fmt}"),
            Tx::Data { level, frames } => {
                let z = frames.iter().filter(|f| f[f.len() - 3] & 0x20 != 0).count();
                format!("L{level}x{}{}", frames.len(), if z > 0 { "z" } else { "" })
            }
            Tx::CwId { .. } => "cw".into(),
        }
    }

    fn air(t: Tx) -> Option<Payload> {
        match t {
            Tx::Ctrl { code, fmt, .. } => Some(Payload::Ctrl(code, fmt.index())),
            Tx::Data { level, frames } => Some(Payload::Data(level.get(), frames.into_iter().map(Some).collect())),
            Tx::CwId { .. } => None,
        }
    }

    fn arrive(l: &mut Link, i: usize, _: Time, t_end: Time, p: Payload) {
        let busy_until = l.st[i].busy_until();
        if busy_until > t_end - SECOND / 2 && busy_until > l.now - SECOND / 10 {
            return;
        }
        let Some(listen) = l.st[i].listen.clone() else { return };
        let t = l.now as f64 / SECOND as f64;
        match &p {
            Payload::Data(level, _) if !listen.levels.iter().any(|l| l.get() == *level) => {
                l.log.push(format!("{t:8.2} {i} not listening for L{level} ({:?})", listen.levels))
            }
            Payload::Ctrl(code, _) if !listen.codes.contains(code) => {
                l.log.push(format!("{t:8.2} {i} code {code} not accepted"))
            }
            _ => {}
        }
        let snr = l.ch.snr;
        match p {
            Payload::Ctrl(code, f) => {
                if listen.codes.contains(&code) {
                    let hits = [31, 15, 14][f];
                    let fmt = Fmt::from_index(f).unwrap();
                    l.st[i].s.on_ctrl(l.now, CtrlIn { code, fmt, hits, snr_norm: snr, t_end });
                }
            }
            Payload::Data(level, frames) => {
                if let Some(level) = listen.levels.iter().copied().find(|l| l.get() == level) {
                    let frames = frames
                        .into_iter()
                        .map(|f| match f {
                            Some(bytes) => FrameIn { ok: true, bytes, snr_db: snr - 6.0 },
                            None => FrameIn { ok: false, bytes: Vec::new(), snr_db: 0.0 },
                        })
                        .collect();
                    l.st[i].s.on_data(l.now, DataIn { level, frames, t_end });
                }
            }
        }
    }
}

fn cfg(call: &str, registered: bool) -> Config {
    let mut c = Config::new(Bandwidth::Hz2300, call);
    c.host.registered = registered;
    c.host.compression = Compression::Off;
    c
}

fn disconnected(l: &Link) -> bool {
    l.st.iter().all(|s| s.host.contains(&HostEvent::Disconnected))
}

fn transfer(ch: Channel, seed: u64, a_bytes: usize, b_bytes: usize, secs: f64) -> Option<Link> {
    transfer_with(ch, seed, payload(a_bytes, 7), payload(b_bytes, 9), Compression::Off, secs)
}

fn transfer_with(ch: Channel, seed: u64, a: Vec<u8>, b: Vec<u8>, comp: Compression, secs: f64) -> Option<Link> {
    let (a_bytes, b_bytes) = (a.len(), b.len());
    let mut ca = cfg("N0CALL", true);
    ca.host.compression = comp;
    let mut l = Link::new(ca, cfg("N1CALL", false), ch, seed);
    if b_bytes > 0 {
        l.st[1].s.send(0, &b);
    }
    l.st[0].s.connect(0, None, "N1CALL");
    l.flush();
    l.run_until(300.0, connected);
    if !connected(&l) {
        let lost = l.log.iter().filter(|x| x.ends_with("lost on air")).count();
        let sent = l.st[0].sent.iter().chain(&l.st[1].sent).filter(|t| t.starts_with('c')).count();
        assert!(lost * 2 > sent, "no link\n{}", l.log.join("\n"));
        return None;
    }
    l.st[0].s.send(l.now, &a);
    let stalled = |l: &Link| l.now - l.st[0].progress.max(l.st[1].progress) > 900 * SECOND;
    l.run_until(secs, |l| (l.st[1].got >= a_bytes && l.st[0].got >= b_bytes) || stalled(l));
    let (ra, rb) = (l.st[1].rx(), l.st[0].rx());
    if ra.len() < a.len() || rb.len() < b.len() {
        check(&l, &ra, &a[..ra.len().min(a.len())]);
        check(&l, &rb, &b[..rb.len().min(b.len())]);
        l.slow = true;
        return Some(l);
    }
    check(&l, &ra, &a);
    check(&l, &rb, &b);
    l.run_until(5.0, |_| false);
    let caller = l.now;
    l.st[0].s.disconnect(caller);
    l.run_until(120.0, disconnected);
    assert!(disconnected(&l), "{}", l.tail(40));
    Some(l)
}

#[test]
fn clean_link_climbs_and_delivers() {
    let l = transfer(CLEAN, 1, 60_000, 0, 300.0).unwrap();
    let top = l.st[0].levels().into_iter().max().unwrap();
    assert!(top >= 14, "caller topped out at L{top}: {:?}", l.st[0].sent);
    assert!(l.st[0].sent.iter().any(|t| t.ends_with("x2")), "never went two-frame");
    assert_eq!(l.st[1].s.state(), State::Listen);
    assert!(l.st[1].host.contains(&HostEvent::Registered(true)));
}

#[test]
fn unregistered_link_stays_at_level_4() {
    let mut l = Link::new(cfg("N0CALL", false), cfg("N1CALL", true), CLEAN, 3);
    l.st[0].s.connect(0, None, "N1CALL");
    l.flush();
    l.run_until(60.0, connected);
    l.st[0].s.send(l.now, &payload(5000, 1));
    l.run_until(600.0, |l| l.st[1].rx().len() >= 5000);
    check(&l, &l.st[1].rx(), &payload(5000, 1));
    assert_eq!(l.st[0].sent[..6], ["c26f0", "L4x1", "c54f1", "c54f0", "c49f0", "L4x1"]);
    assert_eq!(l.st[1].sent[..5], ["c18f1", "c49f2", "c49f2", "c50f2", "c14f2"]);
    assert_eq!(l.st[0].levels().into_iter().max(), Some(4));
}

#[test]
fn answerer_with_queued_data_and_both_directions() {
    transfer(CLEAN, 2, 8_000, 6_000, 400.0).unwrap();
}

#[test]
fn lossy_channel_delivers_exactly() {
    let (mut linked, mut slow) = (0, 0);
    for seed in 1..=3000 {
        let ch = match seed % 3 {
            0 => Channel { snr: 20.0, frame_loss: 0.15, ctrl_loss: 0.1, miss: 0.05 },
            1 => Channel { snr: 10.0, frame_loss: 0.3, ctrl_loss: 0.2, miss: 0.1 },
            _ => Channel { snr: 30.0, frame_loss: 0.05, ctrl_loss: 0.3, miss: 0.02 },
        };
        let sizes = [15_000, 89, 178, 1, 0, 2873, 341 * 4];
        if let Some(l) = transfer(ch, seed, sizes[seed as usize % 7].max(1), sizes[(seed as usize / 7) % 7], 30_000.0) {
            linked += 1;
            slow += usize::from(l.slow);
        }
    }
    assert!(linked >= 2970, "{linked} of 3000 linked");
    assert!(slow <= 30, "{slow} transfers unfinished in time");
}

fn text(n: usize, seed: u64) -> Vec<u8> {
    const W: [&str; 16] = [
        "the",
        "message",
        "Winlink",
        "station",
        "N0CALL",
        "report",
        "weather",
        "net",
        "check-in",
        "73",
        "de",
        "QSL",
        "frequency",
        "tonight",
        "\r\n",
        "Subject: ",
    ];
    sim::text(&W, n, seed)
}

fn data_frames(l: &Link, i: usize) -> (usize, usize) {
    let d = l.st[i].sent.iter().filter(|t| t.starts_with('L'));
    let z = d.clone().filter(|t| t.ends_with('z')).count();
    (d.count(), z)
}

#[test]
fn text_compression_delivers_and_saves_frames() {
    let (a, b) = (text(80_000, 3), text(3_000, 4));
    let raw = transfer_with(CLEAN, 5, a.clone(), b.clone(), Compression::Off, 3000.0).unwrap();
    let z = transfer_with(CLEAN, 5, a, b, Compression::Text, 3000.0).unwrap();
    let ((rn, rz), (zn, zz)) = (data_frames(&raw, 0), data_frames(&z, 0));
    let (rt, zt) = (raw.st[1].progress as f64 / SECOND as f64, z.st[1].progress as f64 / SECOND as f64);
    assert_eq!(rz, 0);
    assert!(zz > 0 && zn <= rn && zt < rt, "raw {rn} frames {rt:.1} s, TEXT {zn} ({zz} compressed) {zt:.1} s");
    assert!(data_frames(&z, 1).1 > 0, "answerer never compressed");
}

#[test]
fn text_and_files_survive_lossy_channels() {
    let mut linked = 0;
    for seed in 1..=400 {
        let ch = match seed % 3 {
            0 => Channel { snr: 20.0, frame_loss: 0.15, ctrl_loss: 0.1, miss: 0.05 },
            1 => Channel { snr: 15.0, frame_loss: 0.2, ctrl_loss: 0.15, miss: 0.05 },
            _ => Channel { snr: 30.0, frame_loss: 0.05, ctrl_loss: 0.3, miss: 0.02 },
        };
        let comp = if seed % 2 == 0 { Compression::Text } else { Compression::Files };
        let sizes = [6000, 89, 1, 0, 2873, 400];
        let mut a = text(sizes[seed as usize % 6].max(1), seed);
        if seed % 5 == 0 {
            a.extend(payload(500, seed));
        }
        let b = text(sizes[(seed as usize / 6) % 6], seed + 1);
        linked += usize::from(transfer_with(ch, seed, a, b, comp, 30_000.0).is_some());
    }
    assert!(linked >= 395, "{linked} of 400 linked");
}

#[test]
fn files_compression_deflates_chunks() {
    let (a, b) = (text(12_000, 8), text(2_000, 9));
    let raw = transfer_with(CLEAN, 6, a.clone(), b.clone(), Compression::Off, 3000.0).unwrap();
    let z = transfer_with(CLEAN, 6, a, b, Compression::Files, 3000.0).unwrap();
    let (rn, zn) = (data_frames(&raw, 0).0, data_frames(&z, 0).0);
    let (rt, zt) = (raw.st[1].progress as f64 / SECOND as f64, z.st[1].progress as f64 / SECOND as f64);
    assert!(zn < rn && zt < rt, "raw {rn} frames {rt:.1} s, FILES {zn} {zt:.1} s");
}

#[test]
fn disconnect_waits_for_staged_files_data() {
    let mut ca = cfg("N0CALL", true);
    ca.host.compression = Compression::Files;
    let mut l = Link::new(ca, cfg("N1CALL", false), CLEAN, 5);
    l.st[0].s.connect(0, None, "N1CALL");
    l.flush();
    l.run_until(300.0, connected);
    assert!(connected(&l));
    let (now, a) = (l.now, text(500, 3));
    l.st[0].s.send(now, &a);
    l.st[0].s.disconnect(now);
    l.flush();
    l.run_until(300.0, disconnected);
    assert!(disconnected(&l), "{}", l.log.join("\n"));
    assert_eq!(l.st[1].rx(), a);
}

fn up(ch: Channel, seed: u64) -> Link {
    let mut l = Link::new(cfg("N0CALL", false), cfg("N1CALL", false), ch, seed);
    l.st[0].s.connect(0, None, "N1CALL");
    l.flush();
    l.run_until(300.0, connected);
    l
}

fn b2f(seed: u64, msgs: &[&[u8]]) -> Link {
    let mut l = up(CLEAN, seed);
    let now = l.now;
    for m in msgs {
        l.st[0].s.send(now, m);
    }
    l.flush();
    l.run_until(120.0, disconnected);
    l
}

#[test]
fn b2f_fq_ends_the_session_with_qrt() {
    let l = b2f(4, &[b"FF\r", b"FQ\r"]);
    assert!(disconnected(&l), "{}", l.log.join("\n"));
    assert_eq!(l.st[1].rx(), b"FF\rFQ\r");
    let s0 = &l.st[0].sent;
    assert_eq!(s0[s0.len() - 2..], ["c3f0", "cw"], "{s0:?}");
    assert!(!s0.iter().rev().take(4).any(|t| t.starts_with("c14")), "{s0:?}");
}

fn scripted(ca: Config, cb: Config, ch: Channel, seed: u64, a: &[u8], b: &[u8], abort: Option<(usize, usize)>) -> Link {
    let mut l = Link::new(ca, cb, ch, seed);
    if !b.is_empty() {
        l.st[1].s.send(0, b);
    }
    l.st[0].s.connect(0, None, "N1CALL");
    l.flush();
    l.run_until(300.0, connected);
    if !connected(&l) {
        return l;
    }
    let now = l.now;
    l.st[0].s.send(now, a);
    l.flush();
    match abort {
        Some((who, n)) => {
            let data = |l: &Link| l.st.iter().flat_map(|s| &s.sent).filter(|t| t.starts_with('L')).count();
            let n = data(&l) + n;
            l.run_until(600.0, |l| data(l) >= n);
            let now = l.now;
            l.st[who].s.abort(now);
        }
        None => {
            l.run_until(600.0, |l| l.st[1].got >= a.len() && l.st[0].got >= b.len());
            l.run_until(5.0, |_| false);
            let now = l.now;
            l.st[0].s.disconnect(now);
        }
    }
    l.flush();
    l.run_until(300.0, |l| l.st.iter().all(|s| s.s.state() == State::Listen));
    l
}

#[test]
fn golden_transcript() {
    let lossy = |seed: u64| match seed % 3 {
        0 => Channel { snr: 20.0, frame_loss: 0.15, ctrl_loss: 0.1, miss: 0.05 },
        1 => Channel { snr: 12.0, frame_loss: 0.25, ctrl_loss: 0.2, miss: 0.1 },
        _ => Channel { snr: 30.0, frame_loss: 0.05, ctrl_loss: 0.3, miss: 0.02 },
    };
    let with = |bw: Bandwidth, call: &str, registered: bool, comp: Compression, chat: bool| {
        let mut c = cfg(call, registered);
        (c.bw, c.host.compression, c.host.chat) = (bw, comp, chat);
        c
    };
    let reg = |comp| with(Bandwidth::Hz2300, "N0CALL", true, comp, false);
    let ans = || cfg("N1CALL", false);
    let mut cases: Vec<(String, Link)> = Vec::new();
    let mut add = |name: String, l: Link| cases.push((name, l));
    add(
        "clean off".into(),
        scripted(reg(Compression::Off), ans(), CLEAN, 1, &payload(10_000, 1), &payload(2_000, 2), None),
    );
    add("clean unregistered".into(), scripted(cfg("N0CALL", false), ans(), CLEAN, 2, &payload(5_000, 3), &[], None));
    for bw in [Bandwidth::Hz500, Bandwidth::Hz2750] {
        let (a, b) =
            (with(bw, "N0CALL", true, Compression::Off, false), with(bw, "N1CALL", false, Compression::Off, false));
        add(format!("clean {bw:?}"), scripted(a, b, CLEAN, 3, &payload(4_000, 4), &payload(1_000, 5), None));
    }
    let (a, b) = (
        with(Bandwidth::Hz2300, "N0CALL", true, Compression::Text, true),
        with(Bandwidth::Hz2300, "N1CALL", false, Compression::Off, true),
    );
    add("clean chat".into(), scripted(a, b, CLEAN, 4, &text(2_000, 1), &text(500, 2), None));
    add(
        "clean text".into(),
        scripted(reg(Compression::Text), ans(), CLEAN, 5, &text(10_000, 3), &text(2_000, 4), None),
    );
    add(
        "clean files".into(),
        scripted(reg(Compression::Files), ans(), CLEAN, 6, &text(8_000, 8), &text(2_000, 9), None),
    );
    let sizes = [6_000, 89, 178, 1, 0, 2873, 1364];
    for seed in 1..=6 {
        let (a, b) = (payload(sizes[seed % 7].max(1), seed as u64), payload(sizes[seed * 3 % 7], seed as u64 + 1));
        add(
            format!("lossy off {seed}"),
            scripted(reg(Compression::Off), ans(), lossy(seed as u64), seed as u64, &a, &b, None),
        );
    }
    for seed in 10..=17 {
        let comp = if seed < 14 { Compression::Text } else { Compression::Files };
        let mut a = text(sizes[seed % 7].max(1), seed as u64);
        if seed % 4 == 0 {
            a.extend(payload(300, seed as u64));
        }
        let b = text(sizes[seed * 3 % 7], seed as u64 + 1);
        add(
            format!("lossy {comp:?} {seed}"),
            scripted(reg(comp), ans(), lossy(seed as u64), seed as u64, &a, &b, None),
        );
    }
    for who in 0..2 {
        for n in [1, 2, 4, 7] {
            let (a, b) = (payload(5_000, 6), payload(1_500, 7));
            let s = scripted(reg(Compression::Off), ans(), CLEAN, 7, &a, &b, Some((who, n)));
            add(format!("clean abort by {who} after {n} frames"), s);
        }
    }
    for seed in 30..=37u64 {
        let abort = Some(((seed % 2) as usize, 1 + seed as usize % 7));
        let comp = [Compression::Off, Compression::Text, Compression::Files][seed as usize % 3];
        let s = scripted(reg(comp), ans(), lossy(seed), seed, &text(6_000, seed), &text(1_500, seed + 1), abort);
        add(format!("lossy {comp:?} abort {abort:?} {seed}"), s);
    }
    for seed in [55, 58, 60, 66, 70, 75u64] {
        let ch = Channel { snr: 25.0, frame_loss: 0.2, ctrl_loss: 0.0, miss: 0.0 };
        let mut l = up(ch, seed);
        let now = l.now;
        l.st[0].s.send(now, &payload(3_000, seed));
        l.flush();
        let doubles = |l: &Link| l.st[0].sent.iter().filter(|t| t.ends_with("x2")).count();
        let k = 1 + seed as usize % 5;
        l.run_until(600.0, |l| doubles(l) >= k);
        let now = l.now;
        l.st[0].s.abort(now);
        l.flush();
        l.run_until(300.0, |l| l.st.iter().all(|s| s.s.state() == State::Listen));
        add(format!("two-frame abort after {k} doubles {seed}"), l);
    }
    add("b2f fq".into(), b2f(4, &[b"FF\r", b"FQ\r"]));
    add("b2f fq alone".into(), b2f(5, &[b"FQ\r"]));
    golden::check("hf_link.txt", &sim::transcript(&cases));
}
