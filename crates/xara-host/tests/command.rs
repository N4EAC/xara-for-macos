use xara_host::command::{Bw, Command, Comp, Drive, Invalid, fix_suffix, parse, valid_call};
use xara_testkit::json::Value;

fn vector(name: &str) -> Value {
    let p = format!("{}/../../vectors/{name}", env!("CARGO_MANIFEST_DIR"));
    xara_testkit::json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"))).unwrap()
}

#[test]
fn parses_the_lab_command_set() {
    assert_eq!(parse("MYCALL N0CALL"), Some(Command::Mycall(Ok(vec!["N0CALL".into()]))));
    assert_eq!(parse("MYCALL n0call n0call-1"), Some(Command::Mycall(Ok(vec!["N0CALL".into(), "N0CALL-1".into()]))));
    assert_eq!(parse("MYCALL N0CALL1R"), Some(Command::Mycall(Ok(vec!["N0CALL1-R".into()]))));
    assert_eq!(parse("MYCALL N0/CALL"), Some(Command::Mycall(Err(Invalid))));
    assert_eq!(
        parse("CONNECT N0CALL n1call"),
        Some(Command::Connect { mycall: "N0CALL".into(), remote: "N1CALL".into() })
    );
    assert_eq!(parse("BW2300"), Some(Command::Bw(Bw::Hz2300)));
    assert_eq!(parse("LISTEN ON"), Some(Command::Listen(true)));
    assert_eq!(parse("LISTEN CQ"), Some(Command::Chat(true)));
    assert_eq!(parse("CHAT OFF"), Some(Command::Chat(false)));
    assert_eq!(parse("COMPRESSION ON"), Some(Command::Compression(Comp::Text)));
    assert_eq!(parse("WINLINK SESSION"), Some(Command::WinlinkSession));
    assert_eq!(parse("ABORT"), Some(Command::Abort));
    assert_eq!(parse("RETRIES 3"), Some(Command::Retries(Ok(3))));
    assert_eq!(parse("RETRIES abc"), Some(Command::Retries(Ok(0))));
    assert_eq!(parse("RETRIES 300"), Some(Command::Retries(Err(Invalid))));
    assert_eq!(parse("RETRIES -1"), Some(Command::Retries(Err(Invalid))));
    assert_eq!(parse("RETRIES 1E1"), Some(Command::Retries(Ok(10))));
    assert_eq!(parse("RETRIES 2.5e+1"), Some(Command::Retries(Ok(25))));
    assert_eq!(parse("RETRIES 1D1"), Some(Command::Retries(Ok(10))));
    assert_eq!(parse("RETRIES 1.5d+1"), Some(Command::Retries(Ok(15))));
    assert_eq!(parse("RETRIES 3E"), Some(Command::Retries(Ok(3))));
    assert_eq!(parse("RETRIES 3D+"), Some(Command::Retries(Ok(3))));
    assert_eq!(parse("TUNE -12dB"), Some(Command::Tune(Drive::Db(-12))));
    assert_eq!(parse("RETRIES 3e-x"), Some(Command::Retries(Ok(3))));
    assert_eq!(parse("RETRIES 1 2"), Some(Command::Retries(Ok(12))));
    assert_eq!(parse("RETRIES 1\t2\u{3000}3"), Some(Command::Retries(Ok(123))));
    assert_eq!(parse("RETRIES 1\u{b}2"), Some(Command::Retries(Ok(1))));
    assert_eq!(parse("RETRIES &H1F"), Some(Command::Retries(Ok(31))));
    assert_eq!(parse("RETRIES &o17"), Some(Command::Retries(Ok(15))));
    assert_eq!(parse("RETRIES &HFFFF"), Some(Command::Retries(Err(Invalid))));
    assert_eq!(parse("RETRIES &H1FFFF"), Some(Command::Retries(Err(Invalid))));
    assert_eq!(parse("RETRIES E5"), Some(Command::Retries(Ok(0))));
    assert_eq!(parse("TUNE ?"), Some(Command::TuneQuery));
    assert_eq!(parse("TUNE -6"), Some(Command::Tune(Drive::Db(-6))));
    for bad in ["", "CONNECT N0CALL", "LISTEN MAYBE", "BW1000", "MYCALL", "FOO BAR", "version", "VERSION ", "BUFFER"] {
        assert_eq!(parse(bad), None, "{bad:?}");
    }
}

#[test]
fn call_rules_match_the_command_vectors() {
    let r = &vector("host/commands.json")["call_rules"];
    for c in r["valid"].as_array().unwrap() {
        let c = c.as_str().unwrap();
        assert!(valid_call(&fix_suffix(c)).is_some(), "{c}");
    }
    for c in r["invalid"].as_array().unwrap() {
        let c = c.as_str().unwrap();
        assert!(valid_call(&fix_suffix(c)).is_none(), "{c}");
    }
    assert_eq!(valid_call("N0CALL-01").as_deref(), Some("N0CALL-01"));
    assert!(valid_call("N0CALL-X").is_some() && valid_call("N0CALL-16").is_some() && valid_call("N0CALL-1A").is_none());
}

#[test]
fn tune_and_drivelevel_arguments() {
    let cases = [
        ("TUNE -6", Command::Tune(Drive::Db(-6))),
        ("TUNE 0", Command::Tune(Drive::Db(0))),
        ("TUNE -30", Command::Tune(Drive::Db(-30))),
        ("TUNE -5.5", Command::Tune(Drive::Db(-6))),
        ("TUNE -0.5", Command::Tune(Drive::Db(0))),
        ("TUNE  -12dB", Command::Tune(Drive::Db(-12))),
        ("TUNE -30.4", Command::Tune(Drive::OutOfRange)),
        ("TUNE 1", Command::Tune(Drive::OutOfRange)),
        ("TUNE ON", Command::Tune(Drive::NotNumeric)),
        ("TUNE ", Command::Tune(Drive::NotNumeric)),
        ("TUNE -", Command::Tune(Drive::NotNumeric)),
        ("TUNE -1e1", Command::Tune(Drive::Db(-10))),
        ("TUNE -3E+2", Command::Tune(Drive::OutOfRange)),
        ("TUNE &HFFFF", Command::Tune(Drive::Db(-1))),
        ("TUNE &HFFFE0", Command::Tune(Drive::OutOfRange)),
        ("TUNE .", Command::Tune(Drive::NotNumeric)),
        ("TUNE OFF", Command::TuneOff),
        ("TUNE off", Command::TuneOff),
        ("TUNE OFF ", Command::TuneOff),
        ("TUNE  OFF", Command::TuneOff),
        ("TUNE Off\t", Command::TuneOff),
        ("TUNE ? ", Command::TuneQuery),
        ("TUNE  ?", Command::TuneQuery),
        ("DRIVELEVEL -12", Command::DriveLevel(Drive::Db(-12))),
        ("DRIVELEVEL -45", Command::DriveLevel(Drive::OutOfRange)),
        ("DRIVELEVEL x", Command::DriveLevel(Drive::NotNumeric)),
    ];
    for (line, want) in cases {
        assert_eq!(parse(line), Some(want), "{line:?}");
    }
    assert_eq!(parse("TUNE OFF"), Some(Command::TuneOff));
    for bad in ["TUNE", "DRIVELEVEL", "tune -5", "drivelevel -5"] {
        assert_eq!(parse(bad), None, "{bad:?}");
    }
}
