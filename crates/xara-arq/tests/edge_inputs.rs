use xara_arq::hf::frames::parse;
use xara_arq::textcomp::compress;
use xara_arq::{fm, hf};
use xara_phy::fm::level::FmBandwidth;
use xara_phy::hf::level::Bandwidth;

#[test]
fn parse_rejects_frames_too_short_for_their_trailer() {
    for f in [&[][..], &[0x02], &[0x00, 0x02]] {
        assert_eq!(parse(f, 0x55), None, "{f:?}");
    }
    for f in [&[0x02, 0x00, 0x00], &[0x03, 0x00, 0x00]] {
        let p = parse(f, 0x55).unwrap();
        assert_eq!((p.data, p.next_level), (None, None), "{f:?}");
    }
    let p = parse(&[0x14, 0x55, 0x00, 0x00, 0x00], 0x55).unwrap();
    assert_eq!((p.last, p.full, p.data), (false, false, Some(Vec::new())));
    let p = parse(&[0x07, 0x02, 0x00, 0x00], 0x55).unwrap();
    assert_eq!((p.last, p.next_level, p.data), (true, Some(0x07), None));
}

#[test]
fn compress_treats_unknown_states_as_the_start_state() {
    let text = b"the message from the station";
    let want = compress(text, 12, 1);
    for state in [0, 9, 200, usize::MAX] {
        assert_eq!(compress(text, 12, state), want, "state {state}");
    }
    assert_eq!(compress(b"", 4, 0), compress(b"", 4, 1));
}

#[test]
fn sessions_without_a_callsign_do_not_panic() {
    let mut cfg = hf::Config::new(Bandwidth::Hz2300, "N0CALL");
    cfg.host.mycalls.clear();
    let mut s = hf::Session::new(cfg);
    s.connect(0, None, "N1CALL");
    assert_eq!(s.state(), hf::State::Calling);
    s.abort(0);
    s.poll(1000 * 48_000);
    while s.pop().is_some() {}

    let mut cfg = fm::Config::new(FmBandwidth::Narrow, "N0CALL");
    cfg.host.mycalls.clear();
    let mut s = fm::Session::new(cfg);
    s.connect(0, None, "N1CALL", &[]);
    while s.pop().is_some() {}
}
