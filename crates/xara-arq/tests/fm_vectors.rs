mod common;

use std::collections::HashMap;
use xara_arq::fm;
use xara_arq::fm::adapt::{
    Heard, as_level, decode_allow, frame_ok_and_decide, frame_ticks, payload, queue_hint, sender_level_after, thr,
    wide_penalty,
};
use xara_arq::fm::timing::{DETECT_HOLD, caller_connect_window, receiver_sync_window, watchdog};
use xara_arq::time::SECOND;
use xara_phy::fm::level::{FmBandwidth, Record};
use xara_testkit::json::Value;

fn mode(v: &Value) -> FmBandwidth {
    if v == "WIDE" { FmBandwidth::Wide } else { FmBandwidth::Narrow }
}

fn ints(v: &Value) -> Vec<i64> {
    v.as_array().unwrap().iter().map(|x| x.as_i64().unwrap()).collect()
}

fn u(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}

fn rec(v: &Value) -> Record {
    Record::from_index(u(v)).unwrap()
}

fn r4(x: f64) -> f64 {
    (x * 1e4).round() / 1e4
}

#[test]
fn level_tables_match_vara() {
    let v = common::json("fm/levels.json");
    let mut n = 0;
    for r in v["records"].as_array().unwrap() {
        let (bw, rec) = (mode(&r["mode"]), rec(&r["record"]));
        let l = bw.level(rec);
        let ctx = format!("{r}");
        assert_eq!(l.frame_bytes(), u(&r["frame_bytes"]), "{ctx}");
        assert_eq!(l.spf(), u(&r["spf"]), "{ctx}");
        assert_eq!(payload(bw, rec, false), u(&r["payload_direct"]), "{ctx}");
        if let Some(p) = r["payload_with_vias"].as_u64() {
            assert_eq!(payload(bw, rec, true), p as usize, "{ctx}");
        }
        assert_eq!(frame_ticks(bw, rec), r["frame_ticks"].as_i64().unwrap(), "{ctx}");
        assert_eq!(thr(bw, rec), r["snr_threshold_db"].as_f64().unwrap(), "{ctx}");
        assert_eq!(decode_allow(bw, rec), r["decode_allow_ticks"].as_i64().unwrap(), "{ctx}");
        assert_eq!(wide_penalty(bw, rec), r["wide_penalty_db"].as_f64().unwrap(), "{ctx}");
        assert_eq!(l.net_bps().round_ties_even() as i64, r["net_bitrate"].as_i64().unwrap(), "{ctx}");
        n += 1;
    }
    assert_eq!(n, 34);
}

#[test]
fn queue_hint_matches_vara() {
    let v = common::json("fm/hint.json");
    let cases = v["cases"].as_array().unwrap();
    for c in cases {
        let h = queue_hint(mode(&c["mode"]), rec(&c["level"]), u(&c["queued_after"]), c["vias"].as_bool().unwrap());
        assert_eq!(u64::from(h), c["hint"].as_u64().unwrap(), "{c}");
    }
    assert_eq!(cases.len(), 77);
}

#[test]
fn ack_decisions_match_vara() {
    let v = common::json("fm/ack.json");
    let mut n = 0;
    for s in v["sequences"].as_array().unwrap() {
        let bw = mode(&s["mode"]);
        let reg = s["registered"].as_bool().unwrap();
        let mut ad = fm::Adaptation::default();
        let mut lvl = 15;
        for st in s["steps"].as_array().unwrap() {
            if st.get("event").is_some() {
                ad.frame_lost();
                lvl = if lvl == 14 || lvl == 15 { 1 } else { (lvl - 1).max(1) };
                continue;
            }
            assert_eq!(lvl, u(&st["level"]), "{st}");
            let h = Heard {
                record: Record::from_index(lvl).unwrap(),
                hint: st["hint"].as_u64().unwrap() as u8,
                complete: st["complete"].as_bool().unwrap(),
            };
            let d = frame_ok_and_decide(bw, &mut ad, h, st["snr"].as_f64().unwrap(), reg);
            let ctx = format!("{} {st}", s["name"]);
            assert_eq!(d.code, st["code"].as_u64().unwrap() as u32, "{ctx}");
            assert_eq!(d.level, rec(&st["next_level"]), "{ctx}");
            assert_eq!(d.stay_reset, st["explicit_stay_reset"].as_bool().unwrap(), "{ctx}");
            assert_eq!(r4(d.held), st["held_snr"].as_f64().unwrap(), "{ctx}");
            for (i, k) in ["m0", "m1", "m2", "m3", "m6"].iter().enumerate() {
                assert_eq!(r4(d.m[i]), st[k].as_f64().unwrap(), "{ctx} {k}");
            }
            assert_eq!(r4(ad.ratio()), st["ratio"].as_f64().unwrap(), "{ctx}");
            lvl = d.level.index();
            n += 1;
        }
    }
    for c in v["sender"].as_array().unwrap() {
        assert_eq!(
            sender_level_after(rec(&c["level"]), c["code"].as_u64().unwrap() as u32),
            rec(&c["next_level"]),
            "{c}"
        );
        n += 1;
    }
    assert!(n >= 70, "{n}");
}

#[test]
fn timing_windows_match_vara() {
    let v = common::json("fm/timing.json");
    let mut n = 0;
    for c in v["cases"].as_array().unwrap() {
        let (bw, vias) = (mode(&c["mode"]), u(&c["vias"]));
        let top = bw.num_levels();
        assert_eq!(caller_connect_window(bw, vias, false), c["caller_connect_window"].as_i64().unwrap());
        assert_eq!(caller_connect_window(bw, vias, true), c["caller_connect_window_activescan"].as_i64().unwrap());
        let mut t = fm::Timing::new(bw, vias);
        t.ack(400, Record::Connect);
        let s = &c["seed_from_23_24_at_400_ticks"];
        assert_eq!(t.window[1..=top].to_vec(), ints(&s["ack_window"]), "{c}");
        assert_eq!(t.ctrl_window(), s["ctrl_window"].as_i64().unwrap(), "{c}");
        assert_eq!(t.window[14..16].to_vec(), ints(&s["ack_window_rec14_15"]), "{c}");
        let mut t = fm::Timing::new(bw, vias);
        t.ctrl(350);
        let s = &c["seed_from_14_at_350_ticks"];
        assert_eq!(t.window[1..=top].to_vec(), ints(&s["ack_window"]), "{c}");
        assert_eq!(t.ctrl_window(), s["ctrl_window"].as_i64().unwrap(), "{c}");
        for w in c["receiver_sync_window"].as_array().unwrap() {
            let (lo, hi) = receiver_sync_window(bw, rec(&w["level"]), vias, false);
            assert_eq!((lo, hi), (w["lo"].as_f64().unwrap(), w["hi"].as_f64().unwrap()), "{c}");
            n += 1;
        }
        for w in c["receiver_sync_window_double"].as_array().into_iter().flatten() {
            let (lo, hi) = receiver_sync_window(bw, rec(&w["level"]), 0, true);
            assert_eq!((lo, hi), (w["lo"].as_f64().unwrap(), w["hi"].as_f64().unwrap()), "{c}");
            n += 1;
        }
        n += 5;
    }
    let bw = FmBandwidth::Narrow;
    let mut t = fm::Timing::new(bw, 0);
    t.ack(400, Record::Connect);
    for s in v["ack_sequence_narrow_direct"].as_array().unwrap() {
        t.ack(s["ack_ticks"].as_i64().unwrap(), rec(&s["level"]));
        assert_eq!(t.mean[1..12].to_vec(), ints(&s["means"]), "{s}");
        assert_eq!(t.window[1..12].to_vec(), ints(&s["windows"]), "{s}");
        assert_eq!(t.window[14..16].to_vec(), ints(&s["rec14_15"]), "{s}");
        assert_eq!(t.ctrl_window(), s["ctrl_window"].as_i64().unwrap(), "{s}");
        n += 1;
    }
    assert!(n >= 60, "{n}");
}

fn zeroed(rule: &str) -> Option<(f64, f64)> {
    let (a, b) = rule.strip_prefix("zero@")?.split_once('-')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

fn lost(rule: &str, fi: usize, nf: usize, single_lost: impl Fn(f64, f64) -> bool) -> bool {
    if rule == "mute" {
        return true;
    }
    let Some((t1, t2)) = zeroed(rule) else { return false };
    let f2 = 5.376;
    if nf == 2 { (t1 < f2 && fi == 0) || (t2 > f2 + 0.6 && fi == 1) } else { single_lost(t1, t2) }
}

fn burst_secs(bw: FmBandwidth, record: Record, lead: usize, frames: usize) -> f64 {
    (lead + frames * bw.level(record).nsym()) as f64 * 1152.0 / 48000.0
}

#[test]
fn queue_hints_match_lab_frames() {
    let v = common::json("fm/lab_frames.json");
    let checks = &v["checks"];
    let mut total: HashMap<String, usize> = HashMap::new();
    for (k, d) in checks["delivered_bytes"].as_object().unwrap() {
        total.insert(k.clone(), u(&d["delivered"]));
    }
    total.insert("f17_poor:ba".into(), 3000);
    let mut delivered: HashMap<String, usize> = HashMap::new();
    let (mut n, mut ok) = (0, 0);
    for c in v["cases"].as_array().unwrap() {
        let rec = rec(&c["record"]);
        let key = format!("{}:{}", c["run"].as_str().unwrap(), c["dir"].as_str().unwrap());
        let Some(&tot) = total.get(&key) else { continue };
        if matches!(rec, Record::Connect | Record::Broadcast) {
            continue;
        }
        let bw = mode(&c["mode"]);
        let rule = c["rule"].as_str().unwrap_or("");
        let fields = c["fields"].as_array().unwrap();
        let nf = fields.len();
        let base = *delivered.get(&key).unwrap_or(&0);
        let mut taken = 0;
        for (fi, f) in fields.iter().enumerate() {
            taken += u(&f["data_len"]);
            let q = tot.saturating_sub(base + taken);
            let h = queue_hint(bw, rec, q, false);
            n += 1;
            ok += usize::from(u64::from(h) == f["hint"].as_u64().unwrap());
            assert_eq!(u64::from(h), f["hint"].as_u64().unwrap(), "{key} idx {} frame {fi} q {q}", c["idx"]);
            let last = q == 0 && (nf == 1 || fi == 1);
            assert_eq!(u64::from(last), f["last"].as_u64().unwrap(), "{key} idx {}", c["idx"]);
        }
        let got: usize = fields
            .iter()
            .enumerate()
            .filter(|(fi, _)| !lost(rule, *fi, nf, |t1, t2| t2 - t1 > 0.1))
            .map(|(_, f)| u(&f["data_len"]))
            .sum();
        *delivered.entry(key).or_insert(0) += got;
    }
    eprintln!("lab hints {ok}/{n}");
    assert_eq!((ok, n), (74, 74));
}

#[test]
fn ack_decisions_match_lab_frames() {
    let v = common::json("fm/lab_frames.json");
    let mut info: HashMap<(String, String), (usize, Vec<bool>)> = HashMap::new();
    for c in v["cases"].as_array().unwrap() {
        let k = (c["run"].as_str().unwrap().to_string(), format!("{:.4}", c["t"].as_f64().unwrap()));
        let lasts = c["fields"].as_array().unwrap().iter().map(|f| f["last"] == 1).collect();
        info.insert(k, (u(&c["leadin"]), lasts));
    }
    let mut ads: HashMap<(String, String), fm::Adaptation> = HashMap::new();
    let (mut n, mut ok) = (0, 0);
    for c in v["checks"]["ack_decisions"]["cases"].as_array().unwrap() {
        let run = c["run"].as_str().unwrap().to_string();
        let bw = if run == "f10_wide" { FmBandwidth::Wide } else { FmBandwidth::Narrow };
        let ad = ads.entry((run.clone(), c["rx"].as_str().unwrap().into())).or_default();
        let obs = c["obs"].as_u64().unwrap() as u32;
        let rec = rec(&c["record"]);
        let nf = u(&c["frames"]);
        let rule = c["rule"].as_str().unwrap_or("");
        n += 1;
        if rule == "mute" {
            ad.frame_lost();
            ok += usize::from(obs == 2);
            continue;
        }
        let (lead, lasts) = info[&(run.clone(), format!("{:.4}", c["t"].as_f64().unwrap()))].clone();
        let dur = burst_secs(bw, rec, lead, nf);
        let good: Vec<bool> = (0..nf).map(|i| !lost(rule, i, nf, |t1, _| t1 <= 0.75 * dur)).collect();
        let sn = c["sn"].as_f64().unwrap();
        let hint = c["hint"].as_u64().unwrap() as u8;
        if nf == 2 {
            if good[0] {
                ad.frame_ok(sn, as_level(rec), bw.num_levels());
            } else {
                ad.frame_lost();
            }
            if !good[1] {
                ad.frame_lost();
            }
        }
        let pred = if nf == 2 && good[0] && !good[1] {
            11
        } else if nf == 1 && !good[0] {
            2
        } else if *lasts.last().unwrap() {
            ad.frame_ok(sn, as_level(rec), bw.num_levels());
            4
        } else {
            frame_ok_and_decide(bw, ad, Heard { record: rec, hint, complete: good[0] }, sn, false).code
        };
        assert_eq!(pred, obs, "{c}");
        ok += usize::from(pred == obs);
    }
    eprintln!("lab ack decisions {ok}/{n}");
    assert_eq!((ok, n), (68, 68));
}

#[test]
fn windows_match_lab_timing() {
    let v = common::json("fm/lab_timing.json");
    let bw = FmBandwidth::Narrow;
    let ev = v["events"].as_array().unwrap();
    let ticks = |s: f64| (s * SECOND as f64) as i64 / fm::timing::TICK + DETECT_HOLD;
    let w3 = caller_connect_window(bw, 0, false);
    let retry = ev.iter().find(|e| e["what"] == "caller CONNECT retry, direct").unwrap();
    let obs = retry["observed_s_after_rec0_end"].as_f64().unwrap();
    assert!((obs - w3 as f64 * fm::timing::TICK as f64 / SECOND as f64 - 0.02).abs() < 0.1, "{obs}");
    let mut t = fm::Timing::new(bw, 0);
    t.ack(400, Record::Connect);
    t.ctrl(128);
    t.ack(129, Record::Start);
    let w2 = t.data_window(Record::from_index(2).unwrap());
    let b = ev.iter().find(|e| e["what"].as_str().unwrap().starts_with("ACK window boundary")).unwrap();
    let mut n = 0;
    for c in b["cases"].as_array().unwrap() {
        let end = c["ack_end_after_data_end_s"].as_f64().unwrap();
        assert_eq!(ticks(end) <= w2, c["accepted"].as_bool().unwrap(), "{c} w2 {w2}");
        n += 1;
    }
    let wd = ev.iter().find(|e| e["what"] == "disconnect watchdog, direct").unwrap();
    assert!((wd["observed_s"].as_f64().unwrap() - watchdog(0) as f64 / SECOND as f64).abs() < 0.2);
    assert_eq!([watchdog(0), watchdog(1), watchdog(2)], [16_400 * 48, 22_400 * 48, 31_400 * 48]);
    assert_eq!(n, 3);
}
