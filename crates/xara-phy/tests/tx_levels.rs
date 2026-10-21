mod common;
use common::*;
use xara_phy::cwid;
use xara_phy::dsp::quantise;
use xara_phy::hf::ctrl::{self, ATU_SILENT_SYMBOLS, ATU_SYMBOLS, Codebook, Fmt, Role, SYMBOL_SAMPLES, leadin_grids};
use xara_phy::hf::level::{Bandwidth, Record};
use xara_phy::hf::tx::{CWID_BACKOFF_DB, Modulator, tune_symbol};
use xara_testkit::json::Value;

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

fn close(a: f64, b: f64, tol: f64, what: &str) {
    assert!((a - b).abs() <= tol * b.abs(), "{what}: {a} vs {b}");
}

fn peak(x: &[f64]) -> f64 {
    x.iter().fold(0.0, |m, v| m.max(v.abs()))
}

fn rms(x: &[f64]) -> f64 {
    (x.iter().map(|v| v * v).sum::<f64>() / x.len() as f64).sqrt()
}

#[test]
fn level_law_rows() {
    let doc = load_doc("tx_levels");
    let grids = leadin_grids();
    let mut cb = Codebook::new(1, [2, 0, 0, 0, 0], Role::Caller, Bandwidth::Hz2300);
    cb.set_connect_codes(2, Role::Caller);
    let mods: Vec<Modulator> = Bandwidth::ALL.into_iter().map(Modulator::new).collect();
    let mut rng = Rng::new(3);
    let mut checked = 0;
    for r in doc["rows"].as_array().unwrap() {
        let w = r["waveform"].as_str().unwrap();
        let d = f(&r["drive_db"]);
        let what = format!("{w} at {d}");
        if let Some(fmt) = w.strip_prefix("ctrl fmt").map(|s| Fmt::from_index(s.parse().unwrap()).unwrap()) {
            let x = ctrl::frame_audio(&grids, &cb, fmt, 7, 0);
            let g = ctrl::gain(fmt, d);
            close(rms(&x), 1.0, 1e-9, &what);
            close(g * 2.0 / fmt.params().norm(), f(&r["tone_amplitude"]), 1e-12, &what);
            close(rms(&x) * g, f(&r["rms"]), 1e-9, &what);
            assert!(peak(&x) * g <= f(&r["peak_bound"]) * (1.0 + 1e-12), "{what}");
        } else if w.starts_with("FSK") {
            for m in &mods {
                for rec in Record::all().filter(|&rec| m.bw.level(rec).is_fsk()) {
                    let fr = random_frame(&mut rng, m.bw.level(rec).frame_bytes() as usize);
                    let x = m.transmission(rec, &[&fr], &mut |n| rng.below(n));
                    close(peak(&x) * m.gain(rec, d), f(&r["tone_amplitude"]), 1e-3, &what);
                    close(rms(&x) * m.gain(rec, d), f(&r["rms"]), 2e-3, &what);
                }
            }
        } else if w.starts_with("CW ID") {
            let g = 10f64.powf((d - CWID_BACKOFF_DB) / 20.0);
            close(1.6 * g, f(&r["amp_1500"]), 1e-12, &what);
            close(peak(&cwid::synth("N1CALL", 0, false)) * g, f(&r["peak"]), 1e-6, &what);
        } else if w.starts_with("TUNE") {
            close(peak(&tune_symbol(d)), f(&r["tone_amplitude"]), 1e-12, &what);
        } else if let Some(rest) = w.strip_prefix("ATU tone (current ctrl format fmt") {
            let fmt = Fmt::from_index(rest[..1].parse().unwrap()).unwrap();
            let x = ctrl::atu_tone(fmt);
            assert_eq!(x.len(), (ATU_SYMBOLS + ATU_SILENT_SYMBOLS) * SYMBOL_SAMPLES);
            close(peak(&x) * ctrl::gain(fmt, d), f(&r["tone_amplitude"]), 1e-12, &what);
            assert_eq!((quantise(peak(&x) * ctrl::gain(fmt, d)) * 32767.0).round(), f(&r["peak_i16"]), "{what}");
        } else {
            let mut p = w.split_whitespace().skip(2);
            let bw: Bandwidth = p.next().unwrap().parse().unwrap();
            let rec = Record::new(p.next().unwrap()[1..].parse().unwrap()).unwrap();
            let m = mods.iter().find(|m| m.bw == bw).unwrap();
            close(m.gain(rec, d), f(&r["gain"]), 1e-6, &what);
            close(bw.amp_norm(rec), f(&r["ampNorm"]), 1e-6, &what);
            if d == -5.0 {
                let fr = random_frame(&mut rng, bw.level(rec).frame_bytes() as usize);
                let x = m.transmission(rec, &[&fr], &mut |n| rng.below(n));
                close(rms(&x), f(&r["rms_expected"]) / f(&r["gain"]), 0.025, &what);
            }
        }
        checked += 1;
    }
    assert_eq!(checked, 5 * 43);
}

#[test]
fn quantise_clips_and_rounds_ties_even() {
    assert_eq!(quantise(1.7), 1.0);
    assert_eq!(quantise(-3.0), -1.0);
    assert_eq!(quantise(2.5 / 32767.0) * 32767.0, 2.0);
    assert_eq!(quantise(3.5 / 32767.0) * 32767.0, 4.0);
}

#[test]
fn atu_tone_shape() {
    let a = &load_doc("tx_atu_tune")["atu_tone"];
    assert_eq!(ATU_SYMBOLS as u64, a["symbols"].as_u64().unwrap());
    assert_eq!(ATU_SILENT_SYMBOLS as u64, a["silent_tail_symbols"].as_u64().unwrap());
    let x = ctrl::atu_tone(Fmt::F1);
    let n = a["samples"].as_u64().unwrap() as usize;
    assert!(x[n..].iter().all(|&v| v == 0.0));
    let k = f(&a["freq_hz"]) / 48000.0 * SYMBOL_SAMPLES as f64;
    let w = 2.0 * std::f64::consts::PI * k / SYMBOL_SAMPLES as f64;
    let want = |i: usize| (w * (i % SYMBOL_SAMPLES) as f64).sin() / std::f64::consts::SQRT_2;
    assert!(x[..n].iter().enumerate().all(|(i, v)| (v - want(i)).abs() < 1e-12));
}
