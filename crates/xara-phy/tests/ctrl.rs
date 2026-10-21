mod common;
use common::*;
use xara_phy::call::call_hash;
use xara_phy::hf::ctrl::{Codebook, Fmt, Role, Symbol, leadin_grids, leadin_symbols, synth};
use xara_phy::hf::level::Bandwidth;
use xara_testkit::json::Value;

fn fmt_symbols(cw: &[Symbol]) -> String {
    cw.iter().map(|t| t.iter().map(u32::to_string).collect::<Vec<_>>().join("+")).collect::<Vec<_>>().join(" ")
}

fn hexes(v: &Value) -> Vec<u32> {
    v.as_array().unwrap().iter().map(hex_u32).collect()
}

#[test]
fn codebooks() {
    for bw in Bandwidth::ALL {
        let c = load(&format!("ctrl_codebooks_{}", bw.hz()));
        let books = c["codebooks"].as_array().unwrap();
        assert!(!books.is_empty());
        for b in books {
            let what = format!("{} {} {}", bw.hz(), b["station"], b["phase"]);
            let h = hexes(&b["mycall_hashes"]);
            let role = if b["role"].as_u64().unwrap() == 0 { Role::Answerer } else { Role::Caller };
            let mut cb = Codebook::new(hex_u32(&b["seed"]), h.try_into().unwrap(), role, bw);
            if !b["connect_seed"].is_null() {
                cb.set_connect_codes(hex_u32(&b["connect_seed"]), role);
            }
            for (side, fmts) in b["codes"].as_object().unwrap() {
                for (fmt, codes) in fmts.as_object().unwrap() {
                    let f = Fmt::from_index(fmt.trim_start_matches("fmt").parse().unwrap()).unwrap();
                    for (code, want) in codes.as_object().unwrap() {
                        let code: u32 = code.parse().unwrap();
                        if code == 45 {
                            continue;
                        }
                        let got = if side == "side0" { cb.rx(f, code) } else { cb.tx(f, code) };
                        assert_eq!(fmt_symbols(got), want.as_str().unwrap(), "{what} {side} {fmt} code {code}");
                    }
                }
            }
        }
    }
}

fn lab() -> Value {
    load_doc("kiss/xara_lab")
}

#[test]
fn unconnected_codes_match_lab() {
    let lab = lab();
    let grids = leadin_grids();
    let codes = lab["announce_codes"].as_object().unwrap();
    assert_eq!(codes.len(), 4);
    for bw in Bandwidth::ALL {
        let cb = Codebook::new(0, [call_hash(b"N0CALL"), 0, 0, 0, 0], Role::Caller, bw);
        let listen = Codebook::new(0, [call_hash(b"N1CALL"), 0, 0, 0, 0], Role::Answerer, bw);
        for (code, a) in codes {
            let code: u32 = code.parse().unwrap();
            let tones = u32s(&a["tones"]);
            let got: Vec<u32> = grids[0][..1].iter().chain(cb.tx(Fmt::F0, code)).map(|s| s[0]).collect();
            assert_eq!(got, tones, "{} code {code}", bw.hz());
            for f in Fmt::ALL {
                assert_eq!(cb.tx(f, code), listen.rx(f, code), "{} code {code} fmt{f}", bw.hz());
            }
        }
    }
}

#[test]
fn spec_vectors() {
    let bw = Bandwidth::Hz2300;
    let (n0, n1) = (call_hash(b"N0CALL"), call_hash(b"N1CALL"));
    let caller = Codebook::new(n1, [n0, 0, 0, 0, 0], Role::Caller, bw);
    let answerer = Codebook::new(n1, [n1, 0, 0, 0, 0], Role::Answerer, bw);
    let s = load("alphabet_table");
    let w = &s["codewords_N0CALL_to_N1CALL_2300"];
    assert_eq!(fmt_symbols(caller.tx(Fmt::F0, 26)), w["caller_tx_code26_fmt0"].as_str().unwrap());
    assert_eq!(fmt_symbols(answerer.tx(Fmt::F1, 18)), w["answerer_tx_code18_fmt1"].as_str().unwrap());
    assert_eq!(fmt_symbols(answerer.tx(Fmt::F2, 1)), w["answerer_tx_code1_fmt2"].as_str().unwrap());
}

#[test]
fn leadin() {
    let c = load("ctrl_leadin_grids");
    let g = leadin_grids();
    for (f, rows) in Fmt::ALL.into_iter().zip(&g) {
        let want: Vec<&str> =
            c["grids"][format!("fmt{f}")].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        let got: Vec<String> = rows.iter().map(|t| fmt_symbols(std::slice::from_ref(t))).collect();
        assert_eq!(got, want, "fmt{f}");
        let n = &c["leadin_symbols"][format!("fmt{f}")];
        assert_eq!(u64::from(leadin_symbols(f, 1, 0)), n["short"].as_u64().unwrap());
        assert_eq!(u64::from(leadin_symbols(f, 49, 0)), n["long_14_21_49_50"].as_u64().unwrap());
        assert_eq!(u64::from(leadin_symbols(f, 1, 9)), n["first_while_calling_+9"].as_u64().unwrap());
        assert_eq!(u64::from(leadin_symbols(f, 1, 7)), n["first_while_answering_+7"].as_u64().unwrap());
    }
}

fn normalise(x: &[f64]) -> Vec<i16> {
    let rms = (x.iter().map(|v| v * v).sum::<f64>() / x.len() as f64).sqrt();
    let n = x.len();
    let ramp = |i: usize| (i as f64 / 240.0).min(1.0);
    x.iter()
        .enumerate()
        .map(|(i, &v)| ((v / (rms + 1e-12) * 0.12) * ramp(i) * ramp(n - 1 - i)).clamp(-0.95, 0.95) as f32)
        .map(|v| (f64::from(v) * 32767.0).round_ties_even() as i16)
        .collect()
}

#[test]
fn audio() {
    let refs = load("audio_2300");
    let grids = leadin_grids();
    for (id, mycall, remote, code, fmt, extra) in [
        ("ctrl_N2CALL_to_N1CALL_code26_fmt0_lead10", "N2CALL", "N1CALL", 26, Fmt::F0, 9),
        ("ctrl_N2CALL_to_N1CALL_code54_fmt1", "N2CALL", "N1CALL", 54, Fmt::F1, 0),
        ("ctrl_N2CALL_to_N1CALL_code49_fmt0", "N2CALL", "N1CALL", 49, Fmt::F0, 0),
    ] {
        let (hm, hr) = (call_hash(mycall.as_bytes()), call_hash(remote.as_bytes()));
        let mut cb = Codebook::new(hr, [hm, 0, 0, 0, 0], Role::Caller, Bandwidth::Hz2300);
        cb.set_connect_codes(hm, Role::Caller);
        let lead = leadin_symbols(fmt, code, extra) as usize;
        let x = normalise(&synth(&grids[fmt.index()][..lead], cb.tx(fmt, code)));
        let bytes: Vec<u8> = x.iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(x.len() as u64, refs[id]["n_samples"].as_u64().unwrap(), "{id}");
        assert_eq!(sha256_hex(&bytes), refs[id]["sha256_int16le"].as_str().unwrap(), "{id}");
    }
}
