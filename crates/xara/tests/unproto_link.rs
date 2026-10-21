mod common;

use common::air::{self, BLOCK};
use common::{captures_dir, hex};
use xara::station::hf::Station;
use xara::station::{Modem, Options, Report};
use xara_arq::hf::session::Config;
use xara_arq::host::HostEvent;
use xara_arq::kiss::Command;
use xara_phy::hf::level::Bandwidth;

type Air = air::Air<Station, 2>;

fn pair(bw: [Bandwidth; 2], noise: f64) -> Air {
    let cfg = |bw, call: &str, seed| Config { seed, ..Config::new(bw, call) };
    Air::pair(
        [
            Station::new(cfg(bw[0], "N0CALL", 3), Options::default()),
            Station::new(cfg(bw[1], "N1CALL", 5), Options::default()),
        ],
        [2400; 2],
        noise,
        |r| matches!(r, Report::Trace(_)),
    )
}

impl Air {
    fn kiss_out(&self, i: usize) -> Vec<(Command, Vec<u8>)> {
        self.events[i]
            .iter()
            .filter_map(|(_, r)| match r {
                Report::Kiss(c, d) => Some((*c, d.clone())),
                _ => None,
            })
            .collect()
    }

    fn lines(&self, i: usize) -> Vec<String> {
        self.events[i]
            .iter()
            .filter_map(|(_, r)| match r {
                Report::Line(l) => Some(l.clone()),
                Report::Host(HostEvent::Sn(_)) => Some("SN".into()),
                Report::Host(HostEvent::Pending) => Some("PENDING".into()),
                Report::Host(HostEvent::CancelPending) => Some("CANCELPENDING".into()),
                Report::Ptt(on) => Some(if *on { "PTT ON" } else { "PTT OFF" }.into()),
                Report::Bitrate { level, tx, .. } => Some(format!("BITRATE {level} {}", if *tx { "TX" } else { "RX" })),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn kiss_frames_between_stations() {
    let mut air = pair([Bandwidth::Hz2300, Bandwidth::Hz2300], 0.002);
    air.st[1].set_kiss_rx(true);
    let text: Vec<u8> = (b"VARA KISS broadcast test 0123456789 ".repeat(3))[..100].to_vec();
    let ax25 = hex("82a0a4a64040e09c60868298986103f0403039323334357a343930332e35304e2f30373230312e3735573e78");
    let ax25_rx = hex("82a0a4a64040609c60868298986103f0403039323334357a343930332e35304e2f30373230312e3735573e78");
    let sent = [(Command::Generic, b"Hello".to_vec()), (Command::Generic, text.clone()), (Command::Ax25, ax25)];
    for (cmd, data) in sent.iter().cloned() {
        assert!(air.st[0].kiss(cmd, data));
    }
    let ok = air.run_until(60.0, |a| a.kiss_out(1).len() == 3);
    assert!(ok, "B got {:?}\nA {:?}\nB {:?}", air.kiss_out(1), air.lines(0), air.lines(1));
    assert_eq!(
        air.kiss_out(1),
        vec![(Command::Generic, b"Hello".to_vec()), (Command::Generic, text), (Command::Ax25, ax25_rx)]
    );
    let a = air.lines(0);
    let first: Vec<&str> = a.iter().take(5).map(String::as_str).collect();
    assert_eq!(first, ["PENDING", "PTT ON", "BITRATE 1 TX", "PTT OFF", "CANCELPENDING"]);
    assert_eq!(a.iter().filter(|l| *l == "BITRATE 4 TX").count(), 3, "{a:?}");
    assert_eq!(a.iter().filter(|l| *l == "PTT ON").count(), 3, "{a:?}");
    let b = air.lines(1);
    assert_eq!(b.iter().filter(|l| *l == "SN").count(), 1 + 3 + 2, "{b:?}");
    assert_eq!(air.st[0].bandwidth(), Bandwidth::Hz2300);

    air.st[0].connect("N0CALL", "N1CALL", &[]);
    let c = HostEvent::Connected {
        caller: "N0CALL".into(),
        answerer: "N1CALL".into(),
        vias: Vec::new(),
        bw: Bandwidth::Hz2300.into(),
    };
    let linked = air.run_until(40.0, |a| (0..2).all(|i| a.time_of(i, &Report::Host(c.clone())).is_some()));
    assert!(linked, "no link after KISS:\nA {:?}\nB {:?}", air.lines(0), air.lines(1));
}

#[test]
fn kiss_needs_a_client_and_cq_needs_chat() {
    let mut air = pair([Bandwidth::Hz500, Bandwidth::Hz2300], 0.002);
    let done = |a: &Air, n: usize| a.lines(0).iter().filter(|l| *l == "CANCELPENDING").count() == n;
    assert!(air.st[0].kiss(Command::Generic, b"nobody listens".to_vec()));
    assert!(air.run_until(25.0, |a| a.st[0].ptt()));
    assert!(!air.st[0].cq_frame("N0CALL", Some("2300")));
    assert!(air.run_until(25.0, |a| done(a, 1)));
    assert!(air.st[0].cq_frame("N0CALL", Some("2300")));
    assert!(!air.st[0].cq_frame("N0CALL", Some("2300")));
    assert!(air.run_until(25.0, |a| done(a, 2)));
    air.run_until(3.0, |_| false);
    assert!(air.kiss_out(1).is_empty());
    assert!(air.lines(1).is_empty(), "{:?}", air.lines(1));
}

#[test]
fn cq_frames_between_stations() {
    let mut air = pair([Bandwidth::Hz2300, Bandwidth::Hz500], 0.002);
    let mut cfg = air.st[1].config().clone();
    cfg.host.chat = true;
    assert!(air.st[1].reconfigure(cfg));
    assert!(!air.st[0].cq_frame("N0C@LL", Some("2300")));
    for (call, bw, line) in
        [("N1CALL-5", "500", "CQFRAME N1CALL-5 500"), ("AB1CDE-15", "2750", "CQFRAME AB1CDE-15 2750")]
    {
        assert!(air.st[0].cq_frame(call, Some(bw)));
        let ok = air.run_until(15.0, |a| a.lines(1).iter().any(|l| l == line));
        assert!(ok, "A {:?}\nB {:?}", air.lines(0), air.lines(1));
        air.run_until(1.0, |_| false);
    }
    let b = air.lines(1);
    assert_eq!(b, ["BITRATE 1 RX", "SN", "CQFRAME N1CALL-5 500", "BITRATE 1 RX", "SN", "CQFRAME AB1CDE-15 2750"]);
    let a = air.lines(0);
    assert_eq!(a[..5], ["PENDING", "PTT ON", "BITRATE 1 TX", "PTT OFF", "CANCELPENDING"]);
}

#[test]
fn dcd_at_500_waits_for_busy_off() {
    let mut air = pair([Bandwidth::Hz500, Bandwidth::Hz500], 0.0);
    air.st[0].set_persistence(1.0);
    air.st[0].set_busy(true);
    assert!(air.st[0].kiss(Command::Generic, b"wait".to_vec()));
    air.run_until(2.0, |_| false);
    assert!(air.time_of(0, &Report::Host(HostEvent::Pending)).is_none());
    let off = air.secs();
    air.st[0].set_busy(false);
    air.run_until(1.0, |a| a.time_of(0, &Report::Host(HostEvent::Pending)).is_some());
    let t = air.time_of(0, &Report::Host(HostEvent::Pending)).unwrap() - off;
    assert!((0.19..0.22).contains(&t), "{t}");
}

fn sends_while_busy(bw: Bandwidth, ignore_dcd: bool) -> bool {
    let mut air = pair([bw, Bandwidth::Hz500], 0.0);
    air.st[0].set_ignore_dcd(ignore_dcd);
    air.st[0].set_busy(true);
    assert!(air.st[0].kiss(Command::Generic, b"now".to_vec()));
    air.step();
    air.time_of(0, &Report::Host(HostEvent::Pending)).is_some_and(|t| t < 0.02)
}

#[test]
fn ignore_dcd_sends_while_busy() {
    assert!(sends_while_busy(Bandwidth::Hz500, true));
}

#[test]
fn dcd_at_2300_does_not_wait() {
    assert!(sends_while_busy(Bandwidth::Hz2300, false));
}

#[test]
#[ignore = "needs the VARA lab captures in $XARA_CAPTURES; run with --release --ignored"]
fn vara_lab_captures() {
    let dir = captures_dir().join("kiss_lab");
    let cq = [
        "CQFRAME N0CALL 2300",
        "CQFRAME N1CALL-5 500",
        "CQFRAME N0CALL 2750",
        "CQFRAME AB1CDE-15 2750",
        "CQFRAME K1A 2300",
        "CQFRAME N0CALL 2300",
    ];
    for (run, want_cq) in
        [("k01_kiss_generic", &[][..]), ("k02_ax25_1to11", &[]), ("k03_ax25_12_z", &[]), ("k04_bw_after_kiss_cq", &cq)]
    {
        let x = xara::wav::read(&dir.join(run).join("in_ab.wav")).unwrap().samples;
        let mut cfg = Config::new(Bandwidth::Hz2300, "N1CALL");
        cfg.host.chat = true;
        let mut st = Station::new(cfg, Options::default());
        st.set_kiss_rx(true);
        let (mut got, mut lines) = (Vec::new(), Vec::new());
        let mut out = vec![0.0; BLOCK];
        for c in x.chunks(BLOCK).chain(std::iter::repeat_n(&[0.0; BLOCK][..], 600)) {
            st.process(c, &mut out[..c.len()]);
            while let Some(r) = st.pop() {
                match r {
                    Report::Kiss(cmd, d) => got.push((cmd.byte(), d)),
                    Report::Line(l) => lines.push(l),
                    _ => {}
                }
            }
        }
        let want = xara_host::kiss::Deframer::default().feed(&std::fs::read(dir.join(run).join("b_kiss.bin")).unwrap());
        assert!(!want.is_empty() || !want_cq.is_empty(), "{run}");
        assert_eq!(got, want, "{run}");
        assert_eq!(lines, want_cq, "{run}");
    }
}
