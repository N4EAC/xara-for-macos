mod common;
use common::Client;
use xara::station::fm::Station;
use xara::station::{Modem, Options};
use xara::tnc::Tnc;
use xara_arq::fm;
use xara_host::server::Host;
use xara_phy::fm::level::FmBandwidth;

type Lab = common::Lab<Station>;

fn lab(bw: FmBandwidth) -> Lab {
    let mk = |call: &str| {
        let st = Station::new(fm::Config::new(bw, call), Options { drive_db: -5.0, ..Options::default() });
        Tnc::new(st, Host::bind("127.0.0.1", 0).unwrap())
    };
    Lab::with(vec![mk("XX0A"), mk("XX0B")])
}

fn first_line(lab: &mut Lab, c: &mut Client, line: &str) -> String {
    let n = c.len();
    c.send(line);
    let lines = c.lines.clone();
    lab.run_until(2.0, move |_| lines.lock().unwrap().len() > n);
    c.since(n).first().cloned().unwrap_or_default()
}

fn without_status(c: &Client) -> Vec<String> {
    c.since(0)
        .into_iter()
        .filter(|l| !l.starts_with("PTT ") && !l.starts_with("BUSY ") && !l.starts_with("BUFFER ") && l != "IAMALIVE")
        .collect()
}

fn subsequence(want: &[&str], got: &[String]) -> bool {
    let mut it = got.iter();
    want.iter().all(|w| it.any(|g| g == w || (w.ends_with(" *") && g.starts_with(&w[..w.len() - 1]))))
}

#[test]
fn fm_command_differences() {
    let mut lab = lab(FmBandwidth::Narrow);
    let mut c = lab.attach(0);
    lab.run_until(0.1, |_| false);
    assert_eq!(first_line(&mut lab, &mut c, "MYCALL N0CALL"), "OK");
    assert_eq!(first_line(&mut lab, &mut c, "VERSION"), "VERSION VARA FM v4.4.0");
    for l in ["BW2300", "BW500", "CWID ON", "GATEWAY N0CALL", "CONNECT N0CALL N1CALL VIA", "CONNECT N0CALL"] {
        assert_eq!(first_line(&mut lab, &mut c, l), "WRONG", "{l}");
    }
    for l in ["CHAT OFF", "COMPRESSION TEXT", "LISTEN ON", "RETRIES 10", "WINLINK SESSION"] {
        assert_eq!(first_line(&mut lab, &mut c, l), "OK", "{l}");
    }
    assert_eq!(first_line(&mut lab, &mut c, "CONNECT N0CALL N1CALL VIA N9DIG"), "OK");
    assert_eq!(lab.tnc[0].station().state(), fm::State::Calling);
    assert_eq!(lab.tnc[0].station().session().vias(), 1);
    let n = c.len();
    assert_eq!(first_line(&mut lab, &mut c, "ABORT"), "OK");
    let closed = |c: &Client| ["DISCONNECTED", "CANCELPENDING"].iter().all(|w| c.since(n).iter().any(|l| l == w));
    assert!(lab.run_until(10.0, |l| l.tnc[0].station().idle() && closed(&c)), "{:?}", c.since(n));
    assert_eq!(first_line(&mut lab, &mut c, "CONNECT N0CALL N1CALL VIA N9DIG N8DIG"), "OK");
}

#[test]
fn fm_link_host_messages() {
    let mut lab = lab(FmBandwidth::Narrow);
    let mut a = lab.attach(0);
    let mut b = lab.attach(1);
    lab.run_until(0.1, |_| false);
    first_line(&mut lab, &mut a, "MYCALL N0CALL");
    first_line(&mut lab, &mut b, "MYCALL N1CALL");
    first_line(&mut lab, &mut b, "LISTEN ON");
    let msg: Vec<u8> = (0..1500u32).map(|i| (i * 7 % 251) as u8).collect();
    b.write(&msg);
    lab.run_until(0.2, |_| false);
    assert_eq!(first_line(&mut lab, &mut a, "CONNECT N0CALL N1CALL"), "OK");
    let bytes = a.bytes.clone();
    assert!(
        lab.run_until(60.0, move |_| bytes.lock().unwrap().len() >= 1500),
        "{:?}\n{:?}",
        without_status(&a),
        without_status(&b)
    );
    assert_eq!(*a.bytes.lock().unwrap(), msg);
    let (ga, gb) = (without_status(&a), without_status(&b));
    assert!(
        subsequence(
            &[
                "OK",
                "UNENCRYPTED LINK",
                "BITRATE (1)  566 bps TX",
                "PENDING",
                "CONNECTED N0CALL N1CALL NARROW",
                "SN *",
                "BITRATE (1)  566 bps RX"
            ],
            &ga
        ),
        "{ga:?}"
    );
    assert!(
        subsequence(
            &[
                "UNENCRYPTED LINK",
                "SN *",
                "PENDING",
                "CONNECTED N0CALL N1CALL NARROW",
                "LINK UNREGISTERED",
                "BITRATE (1)  566 bps TX",
                "BITRATE (2)  1188 bps TX"
            ],
            &gb
        ),
        "{gb:?}"
    );
    a.write(b"hello from the caller");
    let bytes = b.bytes.clone();
    assert!(lab.run_until(60.0, move |_| bytes.lock().unwrap().len() >= 21), "{:?}", without_status(&b));
    assert_eq!(*b.bytes.lock().unwrap(), b"hello from the caller");
    first_line(&mut lab, &mut a, "DISCONNECT");
    assert!(lab.run_until(30.0, |l| l.tnc.iter().all(|t| t.station().idle())));
    lab.run_until(0.2, |_| false);
    assert!(a.saw("DISCONNECTED") && b.saw("DISCONNECTED"));
    assert!(b.since(0).iter().rev().take(3).any(|l| l == "CANCELPENDING"), "{:?}", without_status(&b));
}
