mod common;

use common::air::{self, payload};
use xara::station::fm::Station;
use xara::station::{Modem, Options, Report};
use xara_arq::compression::Compression;
use xara_arq::fm;
use xara_arq::host::HostEvent;
use xara_phy::fm::level::FmBandwidth;

type Air = air::Air<Station, 3>;

fn trio(bw: FmBandwidth, registered: bool) -> Air {
    let cfg = |call: &str, seed| {
        let mut c = fm::Config { seed, ..fm::Config::new(bw, call) };
        (c.host.registered, c.host.compression) = (registered, Compression::Off);
        c
    };
    let mut d = Station::new(cfg("N9DIG", 7), Options::default());
    d.set_digipeater("N2CALL");
    Air::new(
        [Station::new(cfg("N0CALL", 3), Options::default()), d, Station::new(cfg("N1CALL", 5), Options::default())],
        [2400; 3],
        [&[1], &[0, 2], &[1]],
        0.0,
        |r| matches!(r, Report::Host(HostEvent::Buffer(_)) | Report::Busy(_)),
    )
}

#[test]
fn digipeated_link_over_audio() {
    let mut air = trio(FmBandwidth::Narrow, true);
    let (pa, pb) = (payload(1500, 1), payload(700, 2));
    air.st[2].send(&pb);
    assert!(air.st[0].connect("N0CALL", "N1CALL", &["N2CALL".into()]));
    let line = Report::Host(HostEvent::Connected {
        caller: "N0CALL".into(),
        answerer: "N1CALL".into(),
        vias: vec!["N2CALL".into()],
        bw: FmBandwidth::Narrow.into(),
    });
    let linked = air.run_until(40.0, |a| a.has(0, &line) && a.has(2, &line));
    assert!(linked, "no link:\n{}", air.trace());
    assert!(air.st[1].digipeating());
    assert!(air.has(1, &Report::Host(HostEvent::Pending)));
    air.st[0].send(&pa);
    let done = air.run_until(240.0, |a| a.got[2].len() >= pa.len() && a.got[0].len() >= pb.len());
    assert!(done, "stalled at {}/{} {}/{}:\n{}", air.got[2].len(), pa.len(), air.got[0].len(), pb.len(), air.trace());
    assert_eq!(air.got[2], pa);
    assert_eq!(air.got[0], pb);
    assert!(air.got[1].is_empty());
    air.st[0].disconnect();
    let closed = air.run_until(40.0, |a| a.st.iter().all(Station::idle));
    assert!(closed, "no disconnect:\n{}", air.trace());
    assert_eq!(air.st[2].state(), fm::State::Listen);
    assert!(air.has(1, &Report::Host(HostEvent::CancelPending)));
    assert!(!air.has(1, &Report::Host(HostEvent::Disconnected)));
}

#[test]
fn digipeater_ignores_other_paths() {
    let mut air = trio(FmBandwidth::Narrow, true);
    assert!(air.st[0].connect("N0CALL", "N1CALL", &["N7CALL".into()]));
    air.run_until(15.0, |_| false);
    assert!(!air.st[1].digipeating(), "{}", air.trace());
}
