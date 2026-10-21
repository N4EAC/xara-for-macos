mod common;

use xara_arq::hf::timing::Timing;
use xara_phy::hf::level::Bandwidth;
use xara_testkit::json::Value;

fn ints(v: &Value) -> Vec<i64> {
    v.as_array().unwrap().iter().map(|x| x.as_i64().unwrap()).collect()
}

#[test]
fn ack_and_ctrl_timing_matches_vara() {
    let v = common::json("hf/ack_timing.json");
    let mut steps = 0;
    for s in v["sequences"].as_array().unwrap() {
        let bw = common::bw(&s["bw"]);
        let wide = s["chat"].as_bool().unwrap();
        let mut t = Timing::new(bw, wide);
        for st in s["steps"].as_array().unwrap() {
            match st["ev"].as_str().unwrap() {
                "reset" => t.reset(wide),
                "ctrl" => t.ctrl(st["t"].as_i64().unwrap(), st["state"] == 7),
                "ack" => t.ack(common::recv(&st["level"]), st["t"].as_i64().unwrap()),
                e => panic!("{e}"),
            }
            let a = &st["after"];
            let ctx = format!("{} step {steps} {}", s["name"], st["ev"]);
            assert_eq!(t.ctrl_delay, a["ctrl_delay"].as_i64().unwrap(), "{ctx}");
            assert_eq!(t.ctrl_allowance(), a["ctrl_allowance"].as_i64().unwrap(), "{ctx}");
            assert_eq!(t.base_allowance(), a["base_allowance"].as_i64().unwrap(), "{ctx}");
            assert_eq!(t.ack_delay, ints(&a["ack_delay"]), "{ctx}");
            assert_eq!(t.ack_windows, ints(&a["ack_windows"]), "{ctx}");
            assert_eq!(t.count, ints(&a["count"]), "{ctx}");
            assert_eq!(t.unset, a["timing_unset"].as_bool().unwrap(), "{ctx}");
            steps += 1;
        }
    }
    assert!(steps >= 20, "{steps}");
}

#[test]
fn reply_format_matches_vara() {
    use xara_arq::hf::frames::{ReplyFormat, choose_reply_format};
    use xara_phy::hf::ctrl::Fmt;
    let v = common::json("hf/reply_format.json");
    let mut n = 0;
    for s in v["sequences"].as_array().unwrap() {
        let mut r = ReplyFormat::default();
        for (i, st) in s["steps"].as_array().unwrap().iter().enumerate() {
            let rx = Fmt::try_from(st["rx_fmt"].as_u64().unwrap()).unwrap();
            let k = rx.params().k_db;
            let norm = st["raw_snr"].as_f64().unwrap() - (9.4 - f64::from(k));
            let (snr, f) = r.update(norm, st["hits"].as_u64().unwrap() as usize, rx);
            let ctx = format!("{} step {i}", s["name"]);
            assert_eq!(snr, st["snr_held"].as_f64().unwrap() as f32, "{ctx}");
            assert_eq!(f.index() as u64, st["reply_fmt"].as_u64().unwrap(), "{ctx}");
            n += 1;
        }
    }
    for c in v["single"].as_array().unwrap() {
        let f = choose_reply_format(
            c["snr_held"].as_f64().unwrap() as f32,
            c["missed_held"].as_u64().unwrap() as usize,
            Fmt::try_from(c["rx_fmt"].as_u64().unwrap()).unwrap(),
        );
        assert_eq!(f.index() as u64, c["reply_fmt"].as_u64().unwrap(), "{c}");
        n += 1;
    }
    assert!(n >= 40, "{n}");
}

#[test]
fn adaptive_margin_matches_vara() {
    use xara_arq::hf::adapt::{Adaptation, Qam};
    let v = common::json("hf/adapt_margin.json");
    let mut n = 0;
    for s in v["sequences"].as_array().unwrap() {
        let bw = common::bw(&s["bw"]);
        let mut a = Adaptation::new(bw);
        for (i, e) in s["events"].as_array().unwrap().iter().enumerate() {
            match e["ev"].as_str().unwrap() {
                "ok" => a.frame_ok(common::recv(&e["level"])),
                "lost" if e["state"].is_null() => a.frame_lost(common::recv(&e["level"])),
                "lost" => {}
                "level_from_snr" => {
                    a.level_from_snr(-50.0, common::rec(1), 0.0);
                }
                x => panic!("{x}"),
            }
            let f = &e["after"];
            let ctx = format!("{} event {i} {}", s["name"], e["ev"]);
            let close = |x: f64, k: &str| (x - f[k].as_f64().unwrap()).abs() < 2e-5;
            assert!(
                close(a.margin, "A") && close(a.margin_decay, "decay") && close(a.margin_prev, "A_prev"),
                "{ctx}: {a:?} vs {f}"
            );
            assert_eq!(i64::from(a.ok_run), f["n"].as_i64().unwrap(), "{ctx}");
            let qam = match a.qam {
                Qam::None => -1,
                Qam::LostFirst => 0,
                Qam::Received => 1,
            };
            assert_eq!(qam, f["qam"].as_i64().unwrap(), "{ctx}");
            assert_eq!(a.last.get() as i64, f["last"].as_i64().unwrap(), "{ctx}");
            assert_eq!(i64::from(a.failed_climbs), f["climb"].as_i64().unwrap(), "{ctx}");
            assert_eq!(a.last_good.get() as i64, f["good"].as_i64().unwrap(), "{ctx}");
            n += 1;
        }
    }
    assert!(n >= 100, "{n}");
}

#[test]
fn turn_start_and_hint_minimum_match_vara() {
    use xara_arq::hf::frames::hint_table_minimum;
    use xara_arq::hf::turn::{HintOverride, TurnKind, TurnStart, TurnState, turn_start};
    let v = common::json("hf/turn_hold.json");
    for c in v["cases"].as_array().unwrap() {
        let st = &c["state"];
        let t = TurnState {
            answerer_hold: st["is_answerer"].as_bool().unwrap() && st["hold"].as_bool().unwrap(),
            chat: st["chat"].as_bool().unwrap(),
            queued: st["Q"].as_u64().unwrap() as usize,
            since_header_ms: c["now_ms_since_header"].as_i64().unwrap() - st["t_header_ms"].as_i64().unwrap(),
            kind: match st["kind"].as_u64().unwrap() {
                0 => TurnKind::Partial,
                1 => TurnKind::Prompt,
                k => {
                    assert_eq!(k, 2, "{}", c["case"]);
                    TurnKind::Overflow
                }
            },
            offered: common::recv(&st["offered"]),
            next_tx: common::recv(&st["next_tx"]),
            text: st["text_comp"].as_bool().unwrap(),
            last_held: st["last_held"].as_u64().unwrap() as usize,
        };
        let (got, held) = turn_start(common::bw(&c["bw"]), &t);
        let want = match (c["action"].as_str().unwrap(), c["hint_override"].as_u64()) {
            ("49", _) => TurnStart::Hold,
            ("50", _) => TurnStart::Idle,
            (_, None | Some(0)) => TurnStart::Data(HintOverride::Off),
            (_, Some(1)) => TurnStart::Data(HintOverride::Max),
            (_, Some(o)) => {
                assert_eq!(o, 2, "{}", c["case"]);
                TurnStart::Data(HintOverride::TableMinimum)
            }
        };
        assert_eq!(got, want, "{}", c["case"]);
        assert_eq!(held as u64, c["last_held_after"].as_u64().unwrap(), "{}", c["case"]);
    }
    for (b, tabs) in v["hint_table_minimum"].as_object().unwrap() {
        let bw = b.parse::<Bandwidth>().unwrap();
        for (k, text) in [("plain", false), ("text", true)] {
            for (i, h) in tabs[k].as_array().unwrap().iter().enumerate() {
                assert_eq!(
                    u64::from(hint_table_minimum(bw, common::rec(i + 1), text)),
                    h.as_u64().unwrap(),
                    "{b} {k} L{}",
                    i + 1
                );
            }
        }
    }
}

#[test]
fn b2f_snoop_and_start_cap_match_vara() {
    use xara_arq::b2f::B2fSnoop;
    use xara_arq::hf::frames::{StartCap, start_level_cap};
    let v = common::json("hf/b2f_snoop.json");
    for c in v["host_snoop"].as_array().unwrap() {
        let h = c["block_hex"].as_str().unwrap();
        let b = common::hex(h);
        let e = &c["expect"];
        let mut s = B2fSnoop::default();
        s.observe(&b);
        assert_eq!((s.take_quit(), s.fs()), (e["fq_flag_set"] == true, e["fs_flag_set"] == true), "{}", c["name"]);
    }
    for c in v["start_level_cap"].as_array().unwrap() {
        let bw = common::bw(&c["bw"]);
        let cap = StartCap {
            registered: c["registered"] == true,
            fs: c["fs_flag"] == true,
            text: c["comp_text_active"] == true,
            header_sent: c["arq_state"] == 4,
        };
        let hi = start_level_cap(bw, cap);
        assert_eq!(hi.get() as u64, c["expect_hi"].as_u64().unwrap(), "{c}");
    }
}
