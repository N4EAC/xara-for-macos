use xara_host::response::{LinkWidth, Response, shown_call};

fn connected(vias: &[&str], width: LinkWidth) -> String {
    let vias = vias.iter().map(|v| v.to_string()).collect();
    Response::Connected { caller: "N0CALL".into(), answerer: "N1CALL-5".into(), vias, width }.to_string()
}

#[test]
fn wire_text() {
    let cases = [
        (Response::Ok, "OK"),
        (Response::Wrong, "WRONG"),
        (Response::Busy(true), "BUSY ON"),
        (Response::Busy(false), "BUSY OFF"),
        (Response::Ptt(true), "PTT ON"),
        (Response::Ptt(false), "PTT OFF"),
        (Response::Bitrate { level: 4, bps: 566, tx: true }, "BITRATE (4)  566 bps TX"),
        (Response::Bitrate { level: 1, bps: 47, tx: false }, "BITRATE (1)  47 bps RX"),
        (Response::Pending, "PENDING"),
        (Response::CancelPending, "CANCELPENDING"),
        (Response::Disconnected, "DISCONNECTED"),
        (Response::LinkRegistered(true), "LINK REGISTERED"),
        (Response::LinkRegistered(false), "LINK UNREGISTERED"),
        (Response::Buffer(1234), "BUFFER 1234"),
        (Response::Sn(-3.25), "SN -3.2"),
        (Response::Sn(12.0), "SN 12.0"),
        (Response::IAmAlive, "IAMALIVE"),
        (Response::Registered(None), "REGISTERED"),
        (Response::Registered(Some(vec!["N0CALL".into(), "N0CALL-5".into()])), "REGISTERED N0CALL N0CALL-5"),
        (Response::Registered(Some(vec!["N0\u{7}CALL".into()])), "REGISTERED N0?CALL"),
        (Response::EncryptionDisabled, "ENCRYPTION DISABLED"),
        (Response::UnencryptedLink, "UNENCRYPTED LINK"),
        (Response::Tune(-5.0), "TUNE -5"),
        (Response::Tune(0.0), "TUNE 0"),
        (Response::CleanTxBuffer { empty: true }, "CLEANTXBUFFER BUFFEREMPTY"),
        (Response::CleanTxBuffer { empty: false }, "CLEANTXBUFFER FAILED "),
        (Response::Version("VERSION VARA HF v4.9.0"), "VERSION VARA HF v4.9.0"),
        (Response::Line("CQFRAME N0CALL-5".into()), "CQFRAME N0CALL-5"),
    ];
    for (r, want) in cases {
        assert_eq!(r.to_string(), want, "{r:?}");
    }
}

#[test]
fn connected_lines() {
    assert_eq!(connected(&[], LinkWidth::Hz(2300)), "CONNECTED N0CALL N1CALL-5 2300");
    assert_eq!(connected(&[], LinkWidth::Narrow), "CONNECTED N0CALL N1CALL-5 NARROW");
    assert_eq!(connected(&["N2CALL"], LinkWidth::Narrow), "CONNECTED N0CALL N1CALL-5 VIA N2CALL NARROW");
    assert_eq!(connected(&["N2CALL", "N3CALL"], LinkWidth::Wide), "CONNECTED N0CALL N1CALL-5 VIA N2CALL N3CALL WIDE");
}

#[test]
fn shown_call_drops_service_suffix_dash() {
    assert_eq!(shown_call("N0CALLA-R"), "N0CALLAR");
    assert_eq!(shown_call("N0CALL-R"), "N0CALL-R");
    assert_eq!(shown_call("N0CALLA-5"), "N0CALLA-5");
}

#[test]
fn shown_call_survives_non_ascii_calls() {
    let from_air: String = [0xE9u8, b'A', b'A', b'A', b'A', 0xE9, b'A'].iter().map(|&b| b as char).collect();
    assert_eq!(from_air.len(), 9);
    assert_eq!(shown_call(&from_air), "?AAAA?A");
    assert_eq!(shown_call(&format!("{from_air}-R")), "?AAAA?AR");
    let wide: String = ['\u{e9}', 'A', 'A', 'A', 'A', 'A', '-', 'R'].iter().collect();
    assert_eq!(wide.len(), 9);
    assert_eq!(shown_call(&wide), "?AAAAA-R");
}

#[test]
fn calls_from_the_air_cannot_inject_host_lines() {
    let line = Response::Connected {
        caller: "N0CALL\rPTT OFF".into(),
        answerer: "N1 CALL".into(),
        vias: vec!["N2\nCALL".into()],
        width: LinkWidth::Narrow,
    }
    .to_string();
    assert_eq!(line, "CONNECTED N0CALL?PTT?OFF N1?CALL VIA N2?CALL NARROW");
    assert_eq!(Response::Line("CQFRAME A\rDISCONNECTED\u{7f}\u{e9}".into()).to_string(), "CQFRAME A?DISCONNECTED??");
    assert_eq!(Response::Line("CQFRAME N0CALL-5 500".into()).to_string(), "CQFRAME N0CALL-5 500");
}
