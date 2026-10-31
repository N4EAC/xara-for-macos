mod common;
use common::Lab;
use std::io::Write;
use xara::station::{Modem, Options};
use xara::tnc::Tnc;
use xara_host::response::Response;
use xara_host::server::Host;
use xara_phy::fm::level::FmBandwidth;
use xara_ui::telemetry::{Control, Event, Feed, METER_FLOOR_DB, channel};

fn feeds<M: Modem>(lab: &mut Lab<M>) -> Vec<Feed> {
    (0..lab.tnc.len())
        .map(|i| {
            let (t, f) = channel(1 << 16);
            lab.tnc[i].set_telemetry(t);
            f
        })
        .collect()
}

fn events(f: &Feed) -> Vec<Event> {
    f.events.try_iter().map(|e| e.1).collect()
}

#[test]
fn connect_seen_through_feed() {
    let mut lab = Lab::new(2);
    let f = feeds(&mut lab);
    let (mut a, mut b) = (lab.attach(0), lab.attach(1));
    lab.settle();
    a.send("MYCALL XX0A");
    b.send("MYCALL XX0B");
    b.send("LISTEN ON");
    lab.settle();
    let idle = f[0].take();
    assert_eq!((idle.link.state.as_str(), idle.link.peer.as_str(), idle.ptt), ("Listen", "", false));
    assert_eq!(idle.retries, 15);
    assert!(idle.rx.samples() > 0 && idle.rx.rms_dbfs() == METER_FLOOR_DB);
    assert!(events(&f[0]).contains(&Event::Host(Response::Ok)));

    a.send("CONNECT XX0A XX0B");
    let (mut keyed, mut heard, mut calling) = (false, false, false);
    let want = "CONNECTED XX0A XX0B 500";
    let (al, bl) = (a.lines.clone(), b.lines.clone());
    let up = |l: &common::Lines| l.lock().unwrap().iter().any(|l| l == want);
    while !(up(&al) && up(&bl)) {
        assert!(lab.now() < 30.0, "no link: {:?}", a.since(0));
        lab.run_until(0.2, |_| false);
        let (sa, sb) = (f[0].take(), f[1].take());
        keyed |= sa.ptt && sa.tx.peak_dbfs() > -20.0;
        heard |= sb.rx.rms_dbfs() > -40.0;
        calling |= sa.link.state == "Calling" && sa.link.peer == "XX0B" && sa.link.attempts >= 1;
    }
    assert!(keyed && heard && calling);
    lab.run_until(2.0, |_| false);
    let (sa, sb) = (f[0].peek(), f[1].peek());
    assert_eq!((sa.link.peer.as_str(), sb.link.peer.as_str()), ("XX0B", "XX0A"));
    assert_ne!(sb.link.state, "Listen");
    assert!(sb.sn.is_some() && sb.link.afc_hz.is_some_and(f64::is_finite));
    assert!(sa.bitrate.is_some_and(|b| b.bps > 0));

    let ea = events(&f[0]);
    let connected = |e: &Event| matches!(e, Event::Host(Response::Connected { caller, .. }) if caller == "XX0A");
    assert!(ea.iter().any(connected));
    assert!(ea.contains(&Event::Host(Response::Ptt(true))));
    assert!(ea.iter().any(|e| matches!(e, Event::Trace(t) if t.starts_with("tx "))));
    assert!(!ea.iter().any(|e| matches!(e, Event::Trace(t) if t.starts_with(' '))));
    assert!(events(&f[1]).iter().any(|e| matches!(e, Event::Host(Response::Sn(_)))));

    let payload = b"XARA telemetry byte counter";
    a.data.write_all(payload).unwrap();
    assert!(lab.run_until(40.0, |_| f[1].peek().received_bytes == payload.len() as u64));
    assert_eq!(f[1].take().received_bytes, payload.len() as u64);
    assert_eq!(f[1].take().received_bytes, payload.len() as u64);
    let received = events(&f[1]);
    assert!(received.iter().any(|e| matches!(e, Event::Received { bytes, .. } if *bytes > 0)));

    a.send("DISCONNECT");
    let gone = |v: &common::Lines| v.lock().unwrap().iter().any(|l| l == "DISCONNECTED");
    assert!(lab.run_until(40.0, |_| gone(&al) && gone(&bl)));
    lab.settle();
    let s = f[0].peek();
    assert_eq!((s.link.state.as_str(), s.link.peer.as_str(), s.sn, s.bitrate), ("Listen", "", None, None));
    assert_eq!(s.dropped, 0);
}

#[test]
fn unanswered_call_reports_unable_to_connect() {
    let mut lab = Lab::new(2);
    let f = feeds(&mut lab);
    lab.deaf[0] = true;
    let mut a = lab.attach(0);
    lab.settle();
    a.send("MYCALL XX0A");
    a.send("RETRIES 2");
    lab.settle();
    assert_eq!(f[0].peek().retries, 2);
    a.send("CONNECT XX0A XX0B");
    let mut seen = Vec::new();
    while !seen.contains(&Event::UnableToConnect) {
        assert!(lab.now() < 120.0, "{:?}", a.since(0));
        lab.run_until(1.0, |_| false);
        seen.extend(events(&f[0]));
    }
    assert!(!seen.contains(&Event::TimedOut));
    assert_eq!(f[0].peek().link.state, "Listen");
}

#[test]
fn fm_peer_from_both_ends() {
    use xara::station::fm::Station;
    let mk = |call: &str| {
        let cfg = xara_arq::fm::Config::new(FmBandwidth::Narrow, call);
        Tnc::new(Station::new(cfg, Options::default()), Host::bind("127.0.0.1", 0).unwrap())
    };
    let mut lab = Lab::with(vec![mk("XX0A"), mk("XX0B")]);
    let f = feeds(&mut lab);
    let (mut a, mut b) = (lab.attach(0), lab.attach(1));
    lab.settle();
    b.send("LISTEN ON");
    a.send("CONNECT XX0A XX0B");
    lab.settle();
    let s = f[0].peek();
    assert_eq!((s.link.state.as_str(), s.link.peer.as_str(), s.link.afc_hz), ("Calling", "XX0B", None));
    let (al, bl) = (a.lines.clone(), b.lines.clone());
    let up = |l: &common::Lines| l.lock().unwrap().iter().any(|l| l.starts_with("CONNECTED XX0A XX0B"));
    assert!(lab.run_until(30.0, |_| up(&al) && up(&bl)), "{:?}", a.since(0));
    assert_eq!((f[0].peek().link.peer.as_str(), f[1].peek().link.peer.as_str()), ("XX0B", "XX0A"));
    assert!(f[1].peek().sn.is_some());
}

#[test]
fn audio_tap_follows_subscription() {
    let mut lab = Lab::new(2);
    let f = feeds(&mut lab);
    lab.settle();
    assert!(f[0].audio.try_recv().is_err());
    f[0].want_audio(true);
    lab.run_until(0.1, |_| false);
    let blocks: Vec<Vec<f32>> = f[0].audio.try_iter().collect();
    assert!(blocks.len() >= 9 && blocks.iter().all(|b| b.len() == 480));
    let s = f[0].peek();
    assert_eq!((s.link.passband, s.link.bw.as_str()), ((1250.0, 1750.0), "500"));
}

fn controls(f: &Feed) -> Vec<(String, bool)> {
    events(f)
        .into_iter()
        .filter_map(|e| match e {
            Event::Control { what, ok } => Some((what, ok)),
            _ => None,
        })
        .collect()
}

#[test]
fn ui_tune_drive_and_time_limit() {
    let mut lab = Lab::new(1);
    let f = feeds(&mut lab);
    lab.settle();
    f[0].control(Control::Drive(-12.0));
    f[0].control(Control::Tune(true));
    lab.run_until(1.0, |_| false);
    f[0].take();
    lab.run_until(0.5, |_| false);
    let s = f[0].take();
    assert!(s.tuning && s.ptt && s.drive_db == -12.0);
    assert!((s.tx.rms_dbfs() + 18.4).abs() < 0.1, "{}", s.tx.rms_dbfs());
    assert!((s.tune_left - 18.5).abs() < 0.1, "{}", s.tune_left);
    f[0].control(Control::Drive(-6.0));
    lab.run_until(0.2, |_| false);
    f[0].take();
    lab.run_until(0.5, |_| false);
    assert!((f[0].take().tx.rms_dbfs() + 12.4).abs() < 0.1);
    assert!(lab.run_until(20.0, |_| !f[0].peek().tuning));
    assert!((lab.now() - 20.1).abs() < 0.1, "{}", lab.now());
    lab.run_until(0.2, |_| false);
    let s = f[0].peek();
    assert!(!s.ptt && s.tune_left == 0.0);
    assert_eq!(
        controls(&f[0]),
        [
            ("drive -12 dB".into(), true),
            ("tune on".into(), true),
            ("drive -6 dB".into(), true),
            ("tune off after 20 s".into(), true)
        ]
    );
    assert_eq!(lab.tnc[0].station().options().drive_db, -6.0);
}

#[test]
fn ui_drive_is_clamped_and_abort_stops_tune() {
    let mut lab = Lab::new(1);
    let f = feeds(&mut lab);
    lab.settle();
    f[0].control(Control::Drive(-100.0));
    f[0].control(Control::Tune(true));
    lab.run_until(0.2, |_| false);
    assert!(f[0].peek().tuning && f[0].peek().drive_db == -30.0);
    f[0].control(Control::Drive(3.0));
    f[0].control(Control::Abort);
    lab.run_until(0.2, |_| false);
    let s = f[0].peek();
    assert!(!s.tuning && !s.ptt && s.drive_db == 3.0);
    assert_eq!(controls(&f[0]).last(), Some(&("abort".into(), true)));
    f[0].control(Control::Drive(100.0));
    lab.run_until(0.2, |_| false);
    assert_eq!(f[0].peek().drive_db, 10.0);
}

#[test]
fn ui_tune_refused_during_a_call() {
    let mut lab = Lab::new(2);
    let f = feeds(&mut lab);
    let mut a = lab.attach(0);
    lab.settle();
    a.send("CONNECT XX0A XX0B");
    lab.run_until(0.5, |_| false);
    assert_ne!(f[0].peek().link.state, "Listen");
    f[0].control(Control::Tune(true));
    lab.run_until(0.1, |_| false);
    assert!(!f[0].peek().tuning);
    assert!(controls(&f[0]).contains(&("tune on".into(), false)));
}

#[test]
fn fm_ui_tune() {
    use xara::station::fm::Station;
    let cfg = xara_arq::fm::Config::new(FmBandwidth::Narrow, "XX0A");
    let tnc = Tnc::new(
        Station::new(cfg, Options { drive_db: -10.0, ..Options::default() }),
        Host::bind("127.0.0.1", 0).unwrap(),
    );
    let mut lab = Lab::with(vec![tnc]);
    let f = feeds(&mut lab);
    lab.settle();
    f[0].control(Control::Tune(true));
    lab.run_until(0.3, |_| false);
    f[0].take();
    lab.run_until(0.5, |_| false);
    let s = f[0].take();
    assert!(s.tuning && s.ptt && (s.tx.rms_dbfs() + 15.007).abs() < 0.05, "{}", s.tx.rms_dbfs());
    f[0].control(Control::Tune(false));
    lab.run_until(0.3, |_| false);
    f[0].take();
    lab.run_until(0.2, |_| false);
    let s = f[0].take();
    assert!(!s.tuning && !s.ptt && s.tx.rms_dbfs() == METER_FLOOR_DB);
}

#[test]
fn restart_only_when_idle() {
    let mut lab = Lab::new(2);
    let f = feeds(&mut lab);
    lab.deaf[0] = true;
    let mut a = lab.attach(0);
    lab.settle();
    f[0].control(Control::Tune(true));
    f[0].control(Control::Restart);
    lab.run_until(0.1, |_| false);
    assert!(!lab.tnc[0].restart_requested());
    f[0].control(Control::Tune(false));
    a.send("CONNECT XX0A XX0B");
    lab.run_until(0.5, |_| false);
    f[0].control(Control::Restart);
    lab.run_until(0.1, |_| false);
    assert!(!lab.tnc[0].restart_requested());
    a.send("ABORT");
    let ok = lab.run_until(30.0, |l| l.tnc[0].station().idle() && !l.tnc[0].station().ptt());
    assert!(ok, "{:?} ptt {} {:?}", f[0].peek().link.state, lab.tnc[0].station().ptt(), a.since(0));
    f[0].control(Control::Restart);
    lab.run_until(0.2, |_| false);
    assert!(!lab.tnc[0].restart_requested());
    lab.run_until(0.2, |_| false);
    assert!(lab.tnc[0].restart_requested());
    let c = controls(&f[0]);
    assert_eq!(c.iter().filter(|(w, _)| w == "restart").map(|x| x.1).collect::<Vec<_>>(), [false, false, true]);
}

#[test]
fn ui_tune_stops_when_its_page_goes() {
    let mut lab = Lab::new(1);
    let f = feeds(&mut lab);
    let mut a = lab.attach(0);
    lab.settle();
    f[0].control_from(7, Control::Tune(true));
    lab.run_until(0.3, |_| false);
    assert!(f[0].peek().tuning);
    f[0].control_from(8, Control::Left);
    lab.run_until(0.2, |_| false);
    assert!(f[0].peek().tuning);
    f[0].control_from(7, Control::Left);
    lab.run_until(0.5, |_| false);
    let s = f[0].peek();
    assert!(!s.tuning && !s.ptt && s.tune_left == 0.0);
    assert_eq!(controls(&f[0]).last(), Some(&("tune off: the page that started it is gone".into(), true)));

    a.send("TUNE -10");
    lab.run_until(0.3, |_| false);
    assert!(f[0].peek().tuning);
    f[0].control_from(7, Control::Left);
    f[0].control_from(0, Control::Left);
    lab.run_until(0.2, |_| false);
    assert!(f[0].peek().tuning);
    a.send("TUNE OFF");
    lab.run_until(0.5, |_| false);
    assert!(!f[0].peek().tuning);
}

#[test]
fn device_output_meter_uses_supplied_pcm_and_asserted_ptt() {
    let cfg = xara_arq::hf::Config::new(xara_phy::hf::level::Bandwidth::Hz500, "W1AW");
    let mut tnc = Tnc::new(xara::station::hf::Station::new(cfg, Options::default()), Host::disabled());
    let (telemetry, feed) = channel(64);
    tnc.set_telemetry(telemetry);
    tnc.use_output_meter();
    tnc.process(&[0.0; 480], &mut [0.0; 480]);
    tnc.publish_output(&[16384, -16384, 32767, -32768], true);
    let status = feed.take();
    assert!(status.ptt);
    assert_eq!(status.tx.samples(), 4);
    assert_eq!(status.tx.clipped(), 2);
    tnc.process(&[0.0; 480], &mut [0.0; 480]);
    tnc.publish_output(&[], false);
    let status = feed.take();
    assert!(!status.ptt);
    assert_eq!(status.tx.samples(), 0, "generated but unwritten audio is never metered");
}
