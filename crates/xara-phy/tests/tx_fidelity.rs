mod common;
use common::*;
use num_complex::Complex64;
use std::collections::BTreeMap;
use std::f64::consts::PI;
use xara_phy::call::call_hash;
use xara_phy::constellation::constellation;
use xara_phy::hf::ctrl::{Codebook, Role};
use xara_phy::hf::fsk;
use xara_phy::hf::level::{Bandwidth, Pulse, Record, Waveform};
use xara_phy::hf::ofdm::{self, NCP, NFFT, NWIN};
use xara_phy::hf::psk::tx_pulse_taps;
use xara_phy::hf::tx::Modulator;
use xara_phy::rnd::VbRnd;
use xara_testkit::json::Value;

fn idx(v: &Value) -> Vec<Vec<u32>> {
    v.as_array().unwrap().iter().map(u32s).collect()
}

fn dft_peak(x: &[f64], lo: u32, hi: u32) -> u32 {
    let n = x.len() as f64;
    let mag = |k: u32| {
        let w = 2.0 * PI * f64::from(k) / n;
        x.iter().enumerate().map(|(i, &v)| Complex64::from_polar(v, -w * i as f64)).sum::<Complex64>().norm()
    };
    (lo..=hi).max_by(|&a, &b| mag(a).total_cmp(&mag(b))).unwrap()
}

#[derive(Default)]
struct Tally {
    cases: usize,
    states: usize,
    fsk_rows: usize,
    psk_cells: usize,
    psk_k0_misses: usize,
    ofdm_cells: usize,
}

#[test]
fn leadin_draws_follow_vara_rnd_stream() {
    let doc = load_doc("tx_leadin_rnd");
    let mods: BTreeMap<u32, Modulator> = Bandwidth::ALL.into_iter().map(|b| (b.hz(), Modulator::new(b))).collect();
    let mut runs: BTreeMap<&str, Vec<&Value>> = BTreeMap::new();
    for c in doc["cases"].as_array().unwrap() {
        runs.entry(c["run"].as_str().unwrap()).or_default().push(c);
    }
    let mut t = Tally::default();
    for (run, cases) in runs {
        let bw = bw_of(&cases[0]["bw"]);
        let m = &mods[&bw.hz()];
        let role = if run == "v2" { Role::Caller } else { Role::Answerer };
        let caller = call_hash(if role == Role::Caller { b"N1CALL" } else { b"N0CALL" });
        let mut rnd = Codebook::new(0, [0; 5], role, bw).set_connect_codes_tx_rnd(caller, role);
        for case in cases {
            let what = format!("{run} t={} {} L{}", case["t"], case["kind"], case["level"]);
            let want_state = hex_u32(&case["rnd_state_before_hex"]);
            let (rec, nf) = (
                Record::new(case["level"].as_u64().unwrap() as usize).unwrap(),
                case["nframes"].as_u64().unwrap() as usize,
            );
            let extra = case["extra_draws_before"].as_u64().unwrap() as usize;
            if nf == 1 {
                for _ in 0..extra {
                    rnd.rand_int(2);
                }
            }
            let fr = hex_frames(&case["frames_hex"]);
            let refs: Vec<&[u8]> = fr.iter().map(Vec::as_slice).collect();
            let (mut states, mut vals) = (Vec::new(), Vec::new());
            let x = m.transmission(rec, &refs, &mut |n| {
                states.push(rnd.state());
                let v = rnd.draw(n);
                vals.push(v);
                v
            });
            let l = bw.level(rec);
            let lead = l.lead_in() as usize;
            let ndraws = case["ndraws"].as_u64().unwrap() as usize;
            assert_eq!(vals.len(), ndraws + if nf > 1 { extra } else { 0 }, "{what}: draws");
            let k0 = match l.waveform {
                _ if nf == 1 => 0,
                Waveform::Fsk { .. } => lead,
                Waveform::Ofdm { nc, .. } => nc as usize * lead,
                Waveform::Psk { .. } => panic!("{what}: no PSK doubles in the vectors"),
            };
            assert_eq!(states[k0], want_state, "{what}: Rnd state");
            t.states += 1;
            match l.waveform {
                Waveform::Fsk { symbols, sps, first, last, step } => {
                    let want = u32s(&case["lead_t"]);
                    assert_eq!(vals[k0..k0 + lead], want[..], "{what}");
                    let other = &m.tables[rec.index()].other;
                    let ns = 16 * sps as usize;
                    for (i, &tv) in want.iter().enumerate() {
                        let bin = first + (u32::from(other[i + 1]) + step * tv - first) % (last - first + 1);
                        assert_eq!(dft_peak(&x[i * ns..(i + 1) * ns], first, last), bin, "{what}: lead row {i}");
                        t.fsk_rows += 1;
                    }
                    assert_eq!(x.len(), (lead + nf * (symbols as usize - 1)) * ns, "{what}");
                }
                Waveform::Psk { carriers_hz, .. } => {
                    let nc = carriers_hz.len();
                    let cst = constellation(l.bps);
                    for (c, row) in idx(&case["lead_over_ref_idx"]).iter().enumerate() {
                        for (k, &want) in row.iter().enumerate() {
                            let r = cst[vals[nf * nc + k * nc + c] as usize] / cst[vals[c] as usize];
                            let got =
                                (0..cst.len()).min_by(|&a, &b| (cst[a] - r).norm().total_cmp(&(cst[b] - r).norm()));
                            if got == Some(want as usize) {
                                t.psk_cells += 1;
                            } else {
                                assert_eq!(k, 0, "{what}: carrier {c} lead {k}");
                                t.psk_k0_misses += 1;
                            }
                        }
                    }
                }
                Waveform::Ofdm { nc, spf, .. } => {
                    for (r, row) in idx(&case["lead_idx_row_bin"]).iter().enumerate() {
                        for (b, &want) in row.iter().enumerate() {
                            assert_eq!(vals[k0 + b * lead + r], want, "{what}: row {r} bin {b}");
                            t.ofdm_cells += 1;
                        }
                    }
                    assert_eq!(nc as usize, idx(&case["lead_idx_row_bin"])[0].len());
                    assert_eq!(x.len(), (lead + nf * (spf as usize - 1)) * (NFFT + NCP), "{what}");
                }
            }
            t.cases += 1;
        }
    }
    eprintln!(
        "{} cases, {} Rnd states, {} FSK lead rows, {} PSK lead cells (+{} k=0 dropouts), {} OFDM lead cells",
        t.cases, t.states, t.fsk_rows, t.psk_cells, t.psk_k0_misses, t.ofdm_cells
    );
    assert_eq!((t.cases, t.states), (125, 125));
}

#[test]
fn psk_rrc_last_tap_is_zero() {
    let r = &load_doc("tx_rules")["psk_rrc"];
    for (span, key) in [(1.0, "taps_pulse_span_1"), (2.0, "taps_pulse_span_2")] {
        let want: Vec<f64> = r[key].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        let got = tx_pulse_taps(Pulse::Rrc { span });
        assert_eq!(got.len(), want.len(), "{key}");
        for (i, (a, b)) in got.iter().zip(&want).enumerate() {
            assert!((a - b).abs() < 1e-7, "{key}[{i}]: {a} vs {b}");
        }
    }
    assert!(tx_pulse_taps(Pulse::Rc).last().unwrap().abs() < 1e-12);
}

#[test]
fn fsk_ends_with_fall() {
    let rules = &load_doc("tx_rules")["fsk"]["ramp_L"];
    let mut rng = Rng::new(5);
    for bw in Bandwidth::ALL {
        let m = Modulator::new(bw);
        for rec in Record::all().filter(|&r| bw.level(r).is_fsk()) {
            let l = bw.level(rec);
            let ns = 16 * l.sps() as usize;
            let len = rules[ns.to_string()].as_u64().unwrap() as usize;
            let fr = random_frame(&mut rng, l.frame_bytes() as usize);
            let x = m.transmission(rec, &[&fr], &mut |n| rng.below(n));
            let last = fsk::rows(l, &m.tables[rec.index()].other, &m.encode(rec, &fr)).last().copied().unwrap();
            let fall = ofdm::fall(len);
            let a = 2.0 / bw.amp_norm(rec);
            for (j, &v) in x[x.len() - ns..].iter().enumerate() {
                let f = if j >= ns - len { fall[j - (ns - len)] } else { 1.0 };
                let want = a * f * (2.0 * PI * f64::from(last) * j as f64 / ns as f64).sin();
                assert!((v - want).abs() < 1e-9, "{bw:?} L{rec} sample {j}");
            }
        }
    }
}

#[test]
fn ofdm_fades_last_samples_in_place() {
    let mut rng = Rng::new(6);
    for bw in Bandwidth::ALL {
        let m = Modulator::new(bw);
        for rec in Record::all().filter(|&r| matches!(bw.level(r).waveform, Waveform::Ofdm { .. })).take(2) {
            let l = bw.level(rec);
            let fr = random_frame(&mut rng, l.frame_bytes() as usize);
            let x = m.transmission(rec, &[&fr], &mut |n| rng.below(n));
            let sym = NFFT + NCP;
            assert_eq!(x.len(), (l.lead_in() + l.spf() - 1) as usize * sym);
            let start = x.len() - sym;
            for (k, f) in ofdm::fall(NWIN).iter().enumerate() {
                let want = f * x[start + NCP - NWIN + k];
                assert!((x[x.len() - NWIN + k] - want).abs() < 1e-12, "{bw:?} L{rec} {k}");
            }
        }
    }
}

#[test]
fn code_49_draw_and_first_state() {
    let rules = &load_doc("tx_rules")["first_draw_positions"];
    let pos = |s: u32| {
        let mut r = VbRnd::from_state(0);
        (0u32..).find(|_| {
            let hit = r.state() == s;
            r.next_f32();
            hit
        })
    };
    for (run, bw, role) in [
        ("v3", Bandwidth::Hz2300, Role::Answerer),
        ("bw2750", Bandwidth::Hz2750, Role::Answerer),
        ("bw500", Bandwidth::Hz500, Role::Answerer),
        ("v2", Bandwidth::Hz2300, Role::Caller),
    ] {
        let caller = call_hash(if role == Role::Caller { b"N1CALL" } else { b"N0CALL" });
        let mut r = Codebook::new(0, [0; 5], role, bw).set_connect_codes_tx_rnd(caller, role);
        if role == Role::Answerer {
            r.rand_int(2);
        }
        assert_eq!(pos(r.state()).map(|i| u64::from(i) + 1), rules[run].as_u64(), "{run}");
    }
}
