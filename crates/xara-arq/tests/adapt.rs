mod common;

use xara_arq::hf::adapt::{Adaptation, SnrTrend, ack_decision};
use xara_phy::hf::level::Bandwidth;
use xara_testkit::json::Value;

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn replays_recorded_ack_decisions() {
    let v = common::json("xara_ack_replay.json");
    let (mut decisions, mut agree) = (0, 0);
    for run in v["runs"].as_array().unwrap() {
        let name = run["name"].as_str().unwrap();
        let mut tr = SnrTrend::default();
        let mut ad = Adaptation::new(common::bw(&run["bw"]));
        let mut codes = Vec::new();
        for (i, op) in run["ops"].as_array().unwrap().iter().enumerate() {
            let at = format!("{name} op {i}: {op}");
            match op["op"].as_str().unwrap() {
                "trend" => assert!(close(tr.push(f(&op["sn"])), f(&op["out"])), "{at}"),
                "ok" => {
                    ad.frame_ok(common::recv(&op["level"]));
                    assert!(close(ad.margin, f(&op["A"])), "{at}: A {}", ad.margin);
                }
                "lost" => {
                    ad.frame_lost(common::recv(&op["level"]));
                    assert!(close(ad.margin, f(&op["A"])), "{at}: A {}", ad.margin);
                }
                "margins" => {
                    let m = ad.margins(f(&op["sn"]), common::recv(&op["level"]));
                    let want: Vec<f64> = op["m"].as_array().unwrap().iter().map(f).collect();
                    assert!(m.iter().zip(&want).all(|(&a, &b)| close(a, b)), "{at}: {m:?}");
                }
                "decide" => {
                    let m: Vec<f64> = op["m"].as_array().unwrap().iter().map(f).collect();
                    let d = ack_decision(
                        common::bw(&run["bw"]),
                        common::recv(&op["level"]),
                        &m.try_into().unwrap(),
                        op["hint"].as_u64().unwrap() as u8,
                        op["complete"].as_bool().unwrap(),
                        common::recv(&op["M"]),
                    );
                    assert_eq!(
                        (u64::from(d.code), d.level),
                        (op["code"].as_u64().unwrap(), common::recv(&op["new"])),
                        "{at}"
                    );
                    codes.push(d.code);
                }
                o => panic!("{o}"),
            }
        }
        let vara = run["vara"].as_array().unwrap();
        assert_eq!(codes.len(), vara.len(), "{name}");
        decisions += vara.iter().filter(|c| !c.is_null()).count();
        agree += codes.iter().zip(vara).filter(|(c, w)| w.as_u64() == Some(u64::from(**c))).count();
    }
    assert_eq!((decisions, agree), (235, 234));
}

#[test]
fn level_from_snr_connect_reply_thresholds() {
    let mut ad = Adaptation::new(Bandwidth::Hz2300);
    let q = |ad: &mut Adaptation, sn| ad.level_from_snr(sn, common::rec(4), 6.0).get();
    assert_eq!(q(&mut ad, -6.9), 4);
    assert_eq!(q(&mut ad, -7.1), 3);
    assert_eq!(q(&mut ad, -9.7), 3);
    assert_eq!(q(&mut ad, -9.9), 2);
    assert_eq!(q(&mut ad, -11.5), 2);
    assert_eq!(q(&mut ad, -11.7), 1);
}
