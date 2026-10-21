mod common;
use common::*;
use xara_phy::hf::level::{Bandwidth, Level, Record, Waveform};
use xara_phy::hf::tx::Modulator;
use xara_phy::{hf::fsk, hf::ofdm};
use xara_testkit::hex;
use xara_testkit::json::Value;

fn num(v: &Value) -> usize {
    v.as_u64().map(|x| x as usize).unwrap_or_else(|| v.as_str().unwrap().parse().unwrap())
}

#[test]
fn fsk_tone_rows() {
    let mods: Vec<Modulator> = Bandwidth::ALL.into_iter().map(Modulator::new).collect();
    let mut checked = 0;
    for name in ["frames_500", "frames_2300", "frames_2750", "vara_frames"] {
        for f in load(name)["frames"].as_array().unwrap() {
            let Some(want) = f.get("fsk_tones_rows_2_S").filter(|v| !v.is_null()) else { continue };
            let m = mods.iter().find(|m| m.bw.hz() as usize == num(&f["mode"])).unwrap();
            let record = Record::new(num(&f["level"])).unwrap();
            let frame = hex::decode(f["frame_hex"].as_str().unwrap()).unwrap();
            let got = fsk::rows(m.bw.level(record), &m.tables[record.index()].other, &m.encode(record, &frame));
            let want = words(want);
            assert_eq!(got[..want.len()], want[..], "{name} {} L{record} {}", f["mode"], f["kind"]);
            checked += 1;
        }
    }
    assert!(checked > 40, "{checked}");
}

fn vara_order(l: &Level, nframes: usize, old: &[u32]) -> Vec<u32> {
    let lead = l.lead_in() as usize;
    match l.waveform {
        Waveform::Fsk { .. } => old.repeat(nframes),
        Waveform::Psk { .. } => {
            let (nc, block) = (l.nc() as usize, lead + nframes);
            let refs = (0..nframes).flat_map(|f| (0..nc).map(move |c| old[c * block + lead + f]));
            refs.chain((0..lead).flat_map(|k| (0..nc).map(move |c| old[c * block + k]))).collect()
        }
        Waveform::Ofdm { nc, .. } => {
            let nc = nc as usize;
            let set: Vec<u32> = (0..nc).flat_map(|b| (0..lead).map(move |r| old[r * nc + b])).collect();
            set.repeat(if nframes > 1 { 3 } else { 1 })
        }
    }
}

#[test]
fn audio_2300_body() {
    let m = Modulator::new(Bandwidth::Hz2300);
    let refs = load("audio_2300");
    let draws = load("xara_audio_draws");
    let mut checked = 0;
    for (id, case) in draws.as_object().unwrap() {
        let frames = hex_frames(&case["frames_hex"]);
        let refs_frame: Vec<&[u8]> = frames.iter().map(|f| f.as_slice()).collect();
        let rec = Record::new(num(&case["level"])).unwrap();
        let l = m.bw.level(rec);
        let mut it = vara_order(l, frames.len(), &u32s(&case["draws"])).into_iter();
        let mut rand = |n: u32| {
            let v = it.next().expect("ran out of draws");
            assert!(v < n, "{id}: draw {v} >= {n}");
            v
        };
        let x = m.transmission(rec, &refs_frame, &mut rand);
        assert!(it.next().is_none(), "{id}: unused draws");
        let r = &refs[id];
        let n = num(&r["n_samples"]);
        let (skip, tail, (lo, hi)) = match l.waveform {
            Waveform::Fsk { sps, .. } => {
                assert_eq!(x.len(), n, "{id}");
                (l.lead_in() as usize * 16 * sps as usize, 16 * sps as usize, (-99.0, -75.0))
            }
            Waveform::Psk { pulse, .. } => {
                assert_eq!(x.len(), n, "{id}");
                let band = match pulse.span() {
                    0.5 => (-99.0, -75.0),
                    1.0 => (-40.0, -36.0),
                    _ => (-53.0, -48.0),
                };
                (8 * 512 + 2560, 4096, band)
            }
            Waveform::Ofdm { .. } => {
                assert_eq!(x.len() + ofdm::NWIN, n, "{id}");
                (240, 1152, (-99.0, -75.0))
            }
        };
        let every = num(&r["fingerprint_every"]);
        let fp: Vec<f64> = r["fingerprint_int16"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        let pts: Vec<(f64, f64)> = (0..fp.len())
            .map(|j| j * every)
            .filter(|&i| i >= skip && i < x.len() - tail)
            .map(|i| (x[i], fp[i / every]))
            .collect();
        let g = pts.iter().map(|(a, b)| a * b).sum::<f64>() / pts.iter().map(|(a, _)| a * a).sum::<f64>();
        let err =
            pts.iter().map(|(a, b)| (b - g * a).powi(2)).sum::<f64>() / pts.iter().map(|(_, b)| b * b).sum::<f64>();
        let db = 10.0 * err.log10();
        assert!(db > lo && db < hi, "{id}: body residual {db:.1} dB against the reference, want {lo}..{hi}");
        checked += 1;
    }
    assert_eq!(checked, 10);
}
