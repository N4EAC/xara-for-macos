mod golden;
mod sim;

use sim::{Channel, Payload, Sess, check, connected, payload};
use xara_arq::compression::Compression;
use xara_arq::fm;
use xara_arq::host::{HostEvent, LinkBw};
use xara_arq::time::{SECOND, Time};
use xara_phy::fm::frame::Record0;
use xara_phy::fm::level::{FmBandwidth, Record};

type Link = sim::Link<fm::Session>;

const CLEAN: Channel = Channel { snr: 40.0, frame_loss: 0.0, ctrl_loss: 0.0, miss: 0.0 };

impl Sess for fm::Session {
    type Config = fm::Config;
    type Out = fm::Output;
    type Tx = fm::Tx;
    type Listen = fm::Listen;
    type State = fm::State;
    const LATENCY: Time = SECOND / 50;

    fn open(c: fm::Config) -> Self {
        fm::Session::new(c)
    }

    fn next(&mut self) -> Option<fm::Output> {
        self.pop()
    }

    fn split(o: fm::Output) -> sim::Out<Self> {
        match o {
            fm::Output::Listen(l) => sim::Out::Listen(l),
            fm::Output::Host(h) => sim::Out::Host(h),
            fm::Output::Tx(t) => sim::Out::Tx(t),
        }
    }

    fn golden_line(o: &fm::Output) -> String {
        match o {
            fm::Output::Tx(fm::Tx::Data { record, frames, lead }) => {
                format!(
                    "Tx(Data {{ record: {record}, frames: [{}], lead: {lead} }})",
                    frames.iter().map(|f| golden::digest(f)).collect::<Vec<_>>().join(", ")
                )
            }
            fm::Output::Host(HostEvent::Data(d)) => format!("Host(Data({}))", golden::digest(d)),
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

    fn st(&self) -> fm::State {
        self.state()
    }

    fn airtime(&self, t: &fm::Tx) -> (Time, Time) {
        match t {
            fm::Tx::Ctrl { lead, .. } => (((lead + 10) * 1024) as Time, 0),
            fm::Tx::Data { record, frames, lead } => {
                ((((lead + frames.len() * self.bandwidth().level(*record).nsym()) * 1152) as Time), 640)
            }
        }
    }

    fn what(t: &fm::Tx) -> String {
        match t {
            fm::Tx::Ctrl { code, lead } => format!("c{code}l{lead}"),
            fm::Tx::Data { record, frames, .. } => {
                let c = if *record == Record::Start { 5 } else { 4 };
                let z = frames.iter().filter(|f| *record != Record::Connect && f[f.len() - c] & 0x20 != 0).count();
                format!("L{record}x{}{}", frames.len(), if z > 0 { "z" } else { "" })
            }
        }
    }

    fn air(t: fm::Tx) -> Option<Payload> {
        match t {
            fm::Tx::Ctrl { code, lead } => Some(Payload::Ctrl(code, lead)),
            fm::Tx::Data { record, frames, .. } => {
                Some(Payload::Data(record.index(), frames.into_iter().map(Some).collect()))
            }
        }
    }

    fn arrive(l: &mut Link, i: usize, start: Time, end: Time, p: Payload) {
        if l.st[i].busy.iter().any(|&(a, b)| a < end && b > start) {
            return;
        }
        let Some(listen) = l.st[i].listen.clone() else { return };
        let snr = l.ch.snr;
        match p {
            Payload::Ctrl(code, lead) => {
                if listen.codes.contains(&code) {
                    let lead_row = (29 - lead) as u8;
                    l.st[i].s.on_ctrl(l.now, fm::CtrlIn { code, lead_row, t_end: end });
                }
            }
            Payload::Data(index, frames) => {
                let record = Record::from_index(index).unwrap();
                if listen.records.contains(&record) {
                    let frames = frames
                        .into_iter()
                        .map(|f| match f {
                            Some(bytes) => fm::FrameIn { ok: true, bytes, snr_db: snr },
                            None => fm::FrameIn { ok: false, bytes: Vec::new(), snr_db: 0.0 },
                        })
                        .collect();
                    l.st[i].s.on_data(l.now, fm::DataIn { record, frames, t_end: end });
                } else {
                    let t = l.now as f64 / SECOND as f64;
                    l.log.push(format!("{t:8.2} {i} not listening for L{record} ({:?})", listen.records));
                }
            }
        }
    }
}

fn cfg(bw: FmBandwidth, call: &str, registered: bool) -> fm::Config {
    let mut c = fm::Config::new(bw, call);
    (c.host.registered, c.host.compression) = (registered, Compression::Off);
    c
}

fn disconnected(l: &Link) -> bool {
    l.st.iter().all(|s| s.host.contains(&HostEvent::Disconnected) && s.s.state() == fm::State::Listen)
}

struct Run {
    bw: FmBandwidth,
    registered: bool,
    comp: Compression,
    ch: Channel,
    seed: u64,
}

fn transfer(r: Run, a: Vec<u8>, b: Vec<u8>, secs: f64) -> Option<Link> {
    let mut ca = cfg(r.bw, "N0CALL", r.registered);
    ca.host.compression = r.comp;
    let mut l = Link::new(ca, cfg(r.bw, "N1CALL", false), r.ch, r.seed);
    if !b.is_empty() {
        l.st[1].s.send(0, &b);
    }
    l.st[0].s.connect(0, None, "N1CALL", &[]);
    l.flush();
    l.run_until(120.0, connected);
    if !connected(&l) {
        let lost = l.log.iter().filter(|x| x.ends_with("lost on air") || x.ends_with("corrupted")).count();
        assert!(lost >= 2, "no link\n{}", l.log.join("\n"));
        return None;
    }
    let now = l.now;
    l.st[0].s.send(now, &a);
    l.flush();
    l.run_until(secs, |l| (l.st[1].got >= a.len() && l.st[0].got >= b.len()) || !connected_now(l));
    let (ra, rb) = (l.st[1].rx(), l.st[0].rx());
    if ra.len() < a.len() || rb.len() < b.len() {
        check(&l, &ra, &a[..ra.len().min(a.len())]);
        check(&l, &rb, &b[..rb.len().min(b.len())]);
        assert!(r.ch.frame_loss > 0.0 || r.ch.ctrl_loss > 0.0, "clean transfer stalled\n{}", l.tail(80));
        if std::env::var_os("FM_DEBUG").is_some() && DUMPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 3 {
            eprintln!("UNFINISHED\n{}", l.tail(60));
        }
        l.slow = true;
        return Some(l);
    }
    check(&l, &ra, &a);
    check(&l, &rb, &b);
    let now = l.now;
    l.st[0].s.disconnect(now);
    l.flush();
    l.run_until(120.0, disconnected);
    assert!(disconnected(&l), "{}", l.tail(40));
    Some(l)
}

static DUMPED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn connected_now(l: &Link) -> bool {
    l.st.iter().all(|s| s.s.state() != fm::State::Listen)
}

fn run(bw: FmBandwidth, registered: bool) -> Run {
    Run { bw, registered, comp: Compression::Off, ch: CLEAN, seed: 1 }
}

#[test]
fn link_start_matches_lab_sequence() {
    let mut l = up(1);
    assert_eq!(l.st[0].sent, ["L0x1", "c14l28", "c4l1", "c20l1"], "{}", l.log.join("\n"));
    assert_eq!(l.st[1].sent, ["c23l28", "L15x1", "c14l28", "c21l28"]);
    let t: Vec<_> = l.st[1].host.iter().filter(|h| !matches!(h, HostEvent::Sn(_))).cloned().collect();
    assert_eq!(
        t[..4],
        [
            HostEvent::Unencrypted,
            HostEvent::Pending,
            HostEvent::Connected {
                caller: "N0CALL".into(),
                answerer: "N1CALL".into(),
                vias: vec![],
                bw: LinkBw::Fm(FmBandwidth::Narrow)
            },
            HostEvent::Registered(false)
        ]
    );
    l.run_until(200.0, disconnected);
    assert!(disconnected(&l), "{}", l.tail(30));
    let polls = l.st[0].sent.iter().filter(|c| *c == "c20l1").count();
    assert_eq!(polls, 7, "{:?}", l.st[0].sent);
    assert_eq!(l.st[0].sent[l.st[0].sent.len() - 2..], ["c3l1", "c3l1"]);
    assert!(l.st[0].host.contains(&HostEvent::Log("stopped sending due to CMS failure".into())));
}

#[test]
fn unregistered_3k_from_target_follows_lab_ladder() {
    let mut l =
        Link::new(cfg(FmBandwidth::Narrow, "N0CALL", false), cfg(FmBandwidth::Narrow, "N1CALL", false), CLEAN, 2);
    let data = payload(3000, 5);
    l.st[1].s.send(0, &data);
    l.st[0].s.connect(0, None, "N1CALL", &[]);
    l.flush();
    l.run_until(120.0, |l| l.st[0].got >= 3000);
    l.run_until(3.0, |_| false);
    check(&l, &l.st[0].rx(), &data);
    assert_eq!(l.st[1].sent[..7], ["c23l28", "L15x1", "L2x1", "L2x1", "L2x2", "L1x1", "c14l28"], "{}", l.tail(40));
    assert_eq!(l.st[0].sent[1..6], ["c14l28", "c7l1", "c1l1", "c1l1", "c6l1"]);
}

#[test]
fn small_queue_takes_record_14() {
    let mut l =
        Link::new(cfg(FmBandwidth::Narrow, "N0CALL", false), cfg(FmBandwidth::Narrow, "N1CALL", false), CLEAN, 2);
    l.st[1].s.send(0, &payload(100, 5));
    l.st[0].s.connect(0, None, "N1CALL", &[]);
    l.flush();
    l.run_until(60.0, |l| l.st[0].got >= 100);
    assert_eq!(l.st[1].sent[..3], ["c23l28", "L15x1", "L14x1"]);
    assert_eq!(l.st[0].sent[1..4], ["c14l28", "c1l1", "c4l1"]);
}

#[test]
fn clean_links_both_ways() {
    for bw in [FmBandwidth::Narrow, FmBandwidth::Wide] {
        for reg in [false, true] {
            let l = transfer(run(bw, reg), payload(40_000, 1), payload(9_000, 2), 3000.0).unwrap();
            let top = l.st[0].levels().into_iter().filter(|&r| r < 14).max().unwrap();
            let want = if reg { if bw == FmBandwidth::Wide { 13 } else { 11 } } else { 2 };
            assert_eq!(top, want, "{bw:?} reg {reg}\n{:?}", l.st[0].sent);
            assert!(l.st[0].sent.iter().any(|t| t.ends_with("x2")), "never went two-frame");
            assert!(l.st[1].host.contains(&HostEvent::Registered(reg)));
        }
    }
}

#[test]
fn lossy_channels_deliver_exactly() {
    let (mut linked, mut slow) = (0, 0);
    for seed in 1..=600 {
        let ch = match seed % 3 {
            0 => Channel { snr: 25.0, frame_loss: 0.15, ctrl_loss: 0.1, miss: 0.05 },
            1 => Channel { snr: 15.0, frame_loss: 0.3, ctrl_loss: 0.2, miss: 0.1 },
            _ => Channel { snr: 40.0, frame_loss: 0.05, ctrl_loss: 0.3, miss: 0.02 },
        };
        let bw = if seed % 2 == 0 { FmBandwidth::Wide } else { FmBandwidth::Narrow };
        let sizes = [9_000, 89, 335, 1, 0, 2873, 334 * 4];
        let a = payload(sizes[seed as usize % 7].max(1), seed);
        let b = payload(sizes[(seed as usize / 7) % 7], seed + 1);
        let r = Run { bw, registered: seed % 4 < 2, comp: Compression::Off, ch, seed };
        if let Some(l) = transfer(r, a, b, 3000.0) {
            linked += 1;
            slow += usize::from(l.slow);
        }
    }
    assert!(linked >= 520, "{linked} of 600 linked");
    eprintln!("lossy: {linked} linked, {slow} unfinished");
    assert!(slow * 4 <= linked, "{slow} of {linked} transfers unfinished");
}

#[test]
fn mild_loss_completes() {
    let (mut done, mut n) = (0, 0);
    for seed in 1..=200 {
        let ch = Channel { snr: 30.0, frame_loss: 0.05, ctrl_loss: 0.05, miss: 0.01 };
        let bw = if seed % 2 == 0 { FmBandwidth::Wide } else { FmBandwidth::Narrow };
        let r = Run { bw, registered: seed % 3 == 0, comp: Compression::Off, ch, seed: seed + 1000 };
        let (a, b) = (payload(6000, seed), payload(700, seed + 1));
        if let Some(l) = transfer(r, a.clone(), b.clone(), 3000.0) {
            n += 1;
            done += usize::from(l.st[1].rx() == a && l.st[0].rx() == b);
        }
    }
    assert!(n >= 195 && done * 100 >= n * 97, "{done} of {n} complete");
}

fn text(n: usize, seed: u64) -> Vec<u8> {
    const W: [&str; 8] = ["the", "message", "Winlink", "station", "N0CALL", "report", "\r\n", "Subject: "];
    sim::text(&W, n, seed)
}

#[test]
fn compression_on_registered_links() {
    for comp in [Compression::Text, Compression::Files] {
        let (mut linked, mut unfinished) = (0, 0);
        for seed in 1..=40 {
            let ch = if seed == 1 { CLEAN } else { Channel { snr: 30.0, frame_loss: 0.1, ctrl_loss: 0.1, miss: 0.02 } };
            let r = Run { bw: FmBandwidth::Narrow, registered: true, comp, ch, seed };
            let (a, b) = (text(12_000, seed), text(1_500, seed + 9));
            let Some(l) = transfer(r, a.clone(), b.clone(), 3000.0) else { continue };
            linked += 1;
            unfinished += usize::from(l.st[1].rx() != a || l.st[0].rx() != b);
            if seed == 1 {
                assert!(l.st[0].sent.iter().any(|t| t.ends_with('z')) || comp == Compression::Files, "{comp:?}");
                assert_eq!((l.st[1].rx(), l.st[0].rx()), (a, b), "{comp:?} clean");
            }
        }
        assert!(linked >= 38, "{comp:?}: {linked} of 40 linked");
        assert!(unfinished <= 2, "{comp:?}: {unfinished} of {linked} transfers unfinished");
    }
}

#[test]
fn unregistered_link_never_compresses() {
    let r = Run { bw: FmBandwidth::Narrow, registered: false, comp: Compression::Text, ch: CLEAN, seed: 3 };
    let l = transfer(r, text(4000, 1), Vec::new(), 600.0).unwrap();
    assert!(!l.st[0].sent.iter().any(|t| t.ends_with('z')));
}

#[test]
fn connect_to_silence_gives_up_on_the_watchdog() {
    let mut l =
        Link::new(cfg(FmBandwidth::Narrow, "N0CALL", false), cfg(FmBandwidth::Narrow, "N1CALL", false), CLEAN, 1);
    l.st[1].deaf = true;
    l.st[0].s.connect(0, None, "N1CALL", &[]);
    l.flush();
    l.run_until(60.0, |l| l.st[0].host.contains(&HostEvent::Disconnected));
    assert_eq!(l.st[0].sent, ["L0x1"; 5]);
    let t = l.now as f64 / SECOND as f64;
    assert!((t - 16.4).abs() < 0.1, "{t}");
    assert!(l.st[0].host.contains(&HostEvent::TimedOut));
}

#[test]
fn unregistered_via_is_refused_at_the_target() {
    let mut t = fm::Session::new(cfg(FmBandwidth::Narrow, "N1CALL", false));
    let mut r = Record0::connect("N0CALL", "N1CALL", "N9DIG", "", false, false);
    let frame = |r: &Record0| fm::DataIn {
        record: Record::Connect,
        frames: vec![fm::FrameIn { ok: true, bytes: r.to_bytes().unwrap(), snr_db: 30.0 }],
        t_end: SECOND,
    };
    t.on_data(SECOND, frame(&r));
    assert_eq!(t.state(), fm::State::Listen);
    r.hop = 1;
    t.on_data(SECOND, frame(&r));
    let out: Vec<fm::Output> = std::iter::from_fn(|| t.pop()).collect();
    assert!(out.contains(&fm::Output::Tx(fm::Tx::Ctrl { code: 3, lead: 28 })), "{out:?}");
    assert!(out.contains(&fm::Output::Host(HostEvent::Sn(0.0))));
    assert!(!out.iter().any(|o| matches!(o, fm::Output::Host(HostEvent::Connected { .. }))));
    t.on_tx_done(2 * SECOND);
    let out: Vec<fm::Output> = std::iter::from_fn(|| t.pop()).collect();
    assert!(!out.iter().any(|o| matches!(o, fm::Output::Tx(_))), "{out:?}");
    assert!(out.contains(&fm::Output::Host(HostEvent::CancelPending)));
    assert!(!out.contains(&fm::Output::Host(HostEvent::Disconnected)));
    assert_eq!(t.state(), fm::State::Listen);
}

#[test]
fn caller_via_keys_side_zero_by_via1() {
    let mut c = fm::Session::new(cfg(FmBandwidth::Narrow, "N0CALL", false));
    assert!(c.connect(0, None, "N1CALL", &["N9DIG".into()]));
    assert_eq!(c.key(), xara_phy::call::call_hash(b"N9DIG"));
    let out: Vec<fm::Output> = std::iter::from_fn(|| c.pop()).collect();
    let r0 = out
        .iter()
        .find_map(|o| if let fm::Output::Tx(fm::Tx::Data { frames, .. }) = o { Some(frames[0].clone()) } else { None });
    let r0 = Record0::parse(&r0.unwrap()).unwrap();
    assert_eq!((r0.hop, r0.via1.as_str(), r0.registered), (0, "N9DIG", false));
}

#[test]
fn watchdog_ends_a_dead_link_without_qrt() {
    let mut l = up(1);
    l.st[1].deaf = true;
    let t0 = l.now;
    l.run_until(60.0, |l| l.st[0].s.state() == fm::State::Listen);
    let dt = (l.now - t0) as f64 / SECOND as f64;
    assert!(dt > 10.0 && dt < 30.0, "{dt}");
    assert!(l.st[0].host.contains(&HostEvent::TimedOut), "{}", l.tail(20));
    assert!(!l.st[0].sent.last().is_some_and(|c| c.starts_with("c3")));
}

#[test]
fn b2f_fq_ends_the_session() {
    let l = b2f(4, &[b"FF\r", b"FQ\r"]);
    assert!(disconnected(&l), "{}", l.log.join("\n"));
    assert_eq!(l.st[1].rx(), b"FF\rFQ\r");
    let s0 = &l.st[0].sent;
    assert_eq!(s0[s0.len() - 2..], ["c3l1", "c3l1"], "{s0:?}");
}

fn scripted(
    ca: fm::Config,
    cb: fm::Config,
    ch: Channel,
    seed: u64,
    a: &[u8],
    b: &[u8],
    abort: Option<(usize, usize)>,
) -> Link {
    let mut l = Link::new(ca, cb, ch, seed);
    if !b.is_empty() {
        l.st[1].s.send(0, b);
    }
    l.st[0].s.connect(0, None, "N1CALL", &[]);
    l.flush();
    l.run_until(120.0, connected);
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
            l.run_until(600.0, |l| (l.st[1].got >= a.len() && l.st[0].got >= b.len()) || !connected_now(l));
            let now = l.now;
            l.st[0].s.disconnect(now);
        }
    }
    l.flush();
    l.run_until(300.0, |l| l.st.iter().all(|s| s.s.state() == fm::State::Listen));
    l
}

fn up(seed: u64) -> Link {
    let n = FmBandwidth::Narrow;
    let mut l = Link::new(cfg(n, "N0CALL", false), cfg(n, "N1CALL", false), CLEAN, seed);
    l.st[0].s.connect(0, None, "N1CALL", &[]);
    l.flush();
    l.run_until(30.0, |l| l.st.iter().all(|s| s.s.state() == fm::State::Idle));
    l
}

fn b2f(seed: u64, msgs: &[&[u8]]) -> Link {
    let mut l = up(seed);
    let now = l.now;
    for m in msgs {
        l.st[0].s.send(now, m);
    }
    l.flush();
    l.run_until(60.0, disconnected);
    l
}

#[test]
fn golden_transcript() {
    let lossy = |seed: u64| match seed % 3 {
        0 => Channel { snr: 25.0, frame_loss: 0.15, ctrl_loss: 0.1, miss: 0.05 },
        1 => Channel { snr: 15.0, frame_loss: 0.3, ctrl_loss: 0.2, miss: 0.1 },
        _ => Channel { snr: 40.0, frame_loss: 0.05, ctrl_loss: 0.3, miss: 0.02 },
    };
    let with = |bw, call: &str, registered, compression, chat| {
        let mut c = fm::Config::new(bw, call);
        (c.host.registered, c.host.compression, c.host.chat) = (registered, compression, chat);
        c
    };
    let n = FmBandwidth::Narrow;
    let mut cases: Vec<(String, Link)> = Vec::new();
    let mut add = |name: String, l: Link| cases.push((name, l));
    for bw in [FmBandwidth::Narrow, FmBandwidth::Wide] {
        for reg in [false, true] {
            let (a, b) = (with(bw, "N0CALL", reg, Compression::Off, false), cfg(bw, "N1CALL", false));
            let l = scripted(a, b, CLEAN, 1, &payload(8_000, 1), &payload(2_000, 2), None);
            add(format!("clean {bw:?} registered {reg}"), l);
        }
    }
    for comp in [Compression::Text, Compression::Files] {
        let (a, b) = (with(n, "N0CALL", true, comp, false), cfg(n, "N1CALL", false));
        add(format!("clean {comp:?}"), scripted(a, b, CLEAN, 2, &text(8_000, 1), &text(1_500, 2), None));
    }
    let (a, b) = (with(n, "N0CALL", true, Compression::Text, true), with(n, "N1CALL", false, Compression::Off, true));
    add("clean chat".into(), scripted(a, b, CLEAN, 3, &text(1_500, 3), &text(400, 4), None));
    let sizes = [5_000, 89, 335, 1, 0, 2873, 1336];
    for seed in 1..=8u64 {
        let i = seed as usize;
        let bw = if seed % 2 == 0 { FmBandwidth::Wide } else { FmBandwidth::Narrow };
        let (a, b) = (payload(sizes[i % 7].max(1), seed), payload(sizes[i * 3 % 7], seed + 1));
        let c = with(bw, "N0CALL", seed % 4 < 2, Compression::Off, false);
        add(format!("lossy off {seed}"), scripted(c, cfg(bw, "N1CALL", false), lossy(seed), seed, &a, &b, None));
    }
    for seed in 13..=20u64 {
        let i = seed as usize;
        let comp = if seed < 17 { Compression::Text } else { Compression::Files };
        let mut a = text(sizes[i % 7].max(1), seed);
        if seed % 4 == 0 {
            a.extend(payload(300, seed));
        }
        let b = text(sizes[i * 3 % 7], seed + 1);
        let c = with(n, "N0CALL", true, comp, false);
        add(format!("lossy {comp:?} {seed}"), scripted(c, cfg(n, "N1CALL", false), lossy(seed), seed, &a, &b, None));
    }
    for who in 0..2 {
        for k in [1, 2, 4, 7] {
            let c = with(n, "N0CALL", true, Compression::Off, false);
            let l =
                scripted(c, cfg(n, "N1CALL", false), CLEAN, 7, &payload(4_000, 6), &payload(1_500, 7), Some((who, k)));
            add(format!("clean abort by {who} after {k} frames"), l);
        }
    }
    for seed in 30..=37u64 {
        let abort = Some(((seed % 2) as usize, 1 + seed as usize % 7));
        let comp = [Compression::Off, Compression::Text, Compression::Files][seed as usize % 3];
        let c = with(n, "N0CALL", true, comp, false);
        let l =
            scripted(c, cfg(n, "N1CALL", false), lossy(seed), seed, &text(5_000, seed), &text(1_500, seed + 1), abort);
        add(format!("lossy {comp:?} abort {abort:?} {seed}"), l);
    }
    let mut l = Link::new(cfg(n, "N0CALL", false), cfg(n, "N1CALL", false), CLEAN, 1);
    l.st[0].s.connect(0, None, "N1CALL", &[]);
    l.flush();
    l.run_until(200.0, disconnected);
    add("idle polls to qrt".into(), l);
    add("b2f fq".into(), b2f(4, &[b"FF\r", b"FQ\r"]));
    add("b2f fq alone".into(), b2f(5, &[b"FQ\r"]));
    let mut l = up(1);
    l.st[1].deaf = true;
    l.run_until(60.0, |l| l.st[0].s.state() == fm::State::Listen);
    add("watchdog".into(), l);
    golden::check("fm_link.txt", &sim::transcript(&cases));
}
