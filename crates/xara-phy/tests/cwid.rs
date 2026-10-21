use xara_phy::cwid::{UNIT, synth, units};

#[test]
fn n0call_is_76_units() {
    assert_eq!(units("N0CALL").len(), 76);
    assert_eq!(synth("N0CALL", 0, false).len(), 76 * UNIT);
}

#[test]
fn blanks_are_silent_until_first_key_down() {
    let x = synth("N1CALL", 9, false);
    assert_eq!(x.len(), (63 + 74) * UNIT);
    assert!(x[..63 * UNIT].iter().all(|&v| v == 0.0));
    let peak = x.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    assert!((peak - 2.187_664_811).abs() < 1e-6, "{peak}");
    let w = 2.0 * std::f64::consts::PI / UNIT as f64;
    assert!((x[63 * UNIT] - 1.6 * (64.0 * w).sin() - 0.8 * (32.0 * w).sin()).abs() < 1e-12);
    let gap = &x[(63 + 5) * UNIT..(63 + 6) * UNIT];
    assert!(gap.iter().any(|&v| v != 0.0), "750 Hz carries on between elements");
    assert!(synth("N1CALL", 0, true)[5 * UNIT..6 * UNIT].iter().all(|&v| v == 0.0));
}
