mod common;
use common::*;
use xara_phy::fm::code::tables;
use xara_phy::fm::constellation::constellation;
use xara_phy::fm::ctrl::{Codebook, frame_audio};
use xara_phy::fm::level::{FmBandwidth, Record};
use xara_phy::fm::tx::{Modulator, TxOptions, to_i16};
use xara_testkit::json::Value;

fn mode(v: &Value) -> FmBandwidth {
    match v.as_str().unwrap() {
        "NARROW" => FmBandwidth::Narrow,
        "WIDE" => FmBandwidth::Wide,
        m => panic!("{m}"),
    }
}

fn check(id: &str, got: &[i16], want: &Value, every: usize) {
    assert_eq!(got.len() as u64, want["n_samples"].as_u64().unwrap(), "{id}: length");
    let fp: Vec<i64> = want["fingerprint"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
    let worst = got.iter().step_by(every).zip(&fp).map(|(&a, &b)| (i64::from(a) - b).abs()).max().unwrap();
    let bytes: Vec<u8> = got.iter().flat_map(|v| v.to_le_bytes()).collect();
    assert_eq!(sha256_hex(&bytes), want["sha256_int16le"].as_str().unwrap(), "{id}: fingerprint off by {worst} LSB");
}

fn data_pcm(m: &Modulator, case: &Value, drive: f64) -> Vec<i16> {
    let record = Record::from_index(case["record"].as_u64().unwrap() as usize).unwrap();
    let key = case["key"].as_u64().unwrap() as u32;
    let fr = hex_frames(&case["frames_hex"]);
    let refs: Vec<&[u8]> = fr.iter().map(Vec::as_slice).collect();
    let draws = case.get("draws").map(u32s).unwrap_or_default();
    let lead = draws.len() / m.bw.level(record).nc();
    let mut it = draws.into_iter();
    let mut rand = |n: u32| {
        let v = it.next().expect("draws");
        assert!(v < n);
        v
    };
    let opt = TxOptions { drive_db: drive, ..TxOptions::default() };
    to_i16(&m.transmission(record, key, &refs, lead, opt, &mut rand))
}

#[test]
fn fm_lab_transmissions() {
    let v = common::load("fm/fm_tx");
    let mods = [Modulator::new(FmBandwidth::Narrow), Modulator::new(FmBandwidth::Wide)];
    let mut n = 0;
    for c in v["l4"].as_array().unwrap() {
        let m = &mods[usize::from(mode(&c["mode"]) == FmBandwidth::Wide)];
        let id = format!("{} {} #{} rec {}", c["run"], c["dir"], c["idx"], c["record"]);
        check(&id, &data_pcm(m, c, -5.0), c, 1009);
        n += 1;
    }
    assert_eq!(n, 99);
}

#[test]
fn all_records() {
    let v = common::load("fm/fm_tx");
    let mods = [Modulator::new(FmBandwidth::Narrow), Modulator::new(FmBandwidth::Wide)];
    for c in v["levels"].as_array().unwrap() {
        let bw = mode(&c["mode"]);
        let m = &mods[usize::from(bw == FmBandwidth::Wide)];
        let record = Record::from_index(c["record"].as_u64().unwrap() as usize).unwrap();
        let id = format!("{bw:?} rec {record} lead {}", c["lead"]);
        check(&id, &data_pcm(m, c, -5.0), c, 1009);
        let t = &tables(bw)[record.index()];
        let w = u32s(&c["perm_entry_bytes"]);
        assert_eq!(w, [2 + u32::from(t.chan.len() > 65536), 2 + u32::from(t.turbo.len() > 65536)], "{id}");
    }
}

#[test]
fn ctrl_frames() {
    let v = common::load("fm/fm_tx");
    for c in v["ctrl"].as_array().unwrap() {
        let cb = Codebook::for_link(
            c["upstream"].as_str().unwrap().as_bytes(),
            c["downstream"].as_str().unwrap().as_bytes(),
        );
        let (code, lead) = (c["code"].as_u64().unwrap() as u32, c["lead"].as_u64().unwrap() as usize);
        check(&format!("code {code} lead {lead}"), &to_i16(&frame_audio(&cb, code, lead, -5.0)), c, 1009);
    }
}

#[test]
fn constellations() {
    let doc = load_doc("fm_constellations");
    for (name, c) in doc["constellations"].as_object().unwrap() {
        let bps = c["bps"].as_u64().unwrap() as u32;
        let pts = c["points"].as_array().unwrap();
        let t = constellation(bps);
        assert_eq!(t.len(), pts.len(), "{name}");
        for (v, (p, z)) in pts.iter().zip(t).enumerate() {
            let (i, q) = (p[0].as_f64().unwrap(), p[1].as_f64().unwrap());
            let tol = if bps <= 2 { 1e-14 } else { 0.0 };
            assert!((z.re - i).abs() <= tol && (z.im - q).abs() <= tol, "{name} v {v}: {z} vs ({i}, {q})");
        }
    }
}

#[test]
fn levels_table() {
    let doc = load_doc("fm/levels");
    for r in doc["records"].as_array().unwrap() {
        let bw = mode(&r["mode"]);
        let rec = Record::from_index(r["record"].as_u64().unwrap() as usize).unwrap();
        let l = bw.level(rec);
        let id = format!("{bw:?} {rec}");
        assert_eq!(l.frame_bytes() as u64, r["frame_bytes"].as_u64().unwrap(), "{id}");
        assert_eq!(l.spf() as u64, r["spf"].as_u64().unwrap(), "{id}");
        assert_eq!(u64::from(l.frame_ticks()), r["frame_ticks"].as_u64().unwrap(), "{id}");
        assert_eq!(
            xara_phy::fm::frame::payload_len(rec, l.frame_bytes(), false).unwrap() as u64,
            r["payload_direct"].as_u64().unwrap(),
            "{id}"
        );
        if let Some(p) = r["payload_with_vias"].as_u64() {
            assert_eq!(xara_phy::fm::frame::payload_len(rec, l.frame_bytes(), true).unwrap() as u64, p, "{id}");
        }
        assert!((f64::from(l.snr_thr_db) - r["snr_threshold_db"].as_f64().unwrap()).abs() < 1e-6, "{id}");
        assert!((f64::from(l.wide_penalty_db) - r["wide_penalty_db"].as_f64().unwrap()).abs() < 1e-6, "{id}");
        if let Some(u) = r["decode_allow_ticks"].as_u64() {
            assert_eq!(u64::from(l.decode_allow), u, "{id}");
        }
        if let Some(b) = r["net_bitrate"].as_u64() {
            assert_eq!(l.net_bps().round_ties_even() as u64, b, "{id}");
        }
        let used = xara_phy::turbo::used_len(l.layout.k as usize, l.puncture_period() as usize);
        assert!(used <= l.n() && l.n() - used <= 2, "{id}: {used} of {}", l.n());
    }
}

#[test]
fn tune_block_is_vara_fm_tune_tone() {
    use xara_phy::fm::tx::{NFFT, TUNE_BACKOFF_DB, tune_block};
    for d in [-30.0, -5.0, 0.0] {
        let x = tune_block(d);
        assert_eq!(x.len(), NFFT);
        let rms = (x.iter().map(|v| v * v).sum::<f64>() / NFFT as f64).sqrt();
        assert!((20.0 * rms.log10() - (d - TUNE_BACKOFF_DB)).abs() < 1e-9);
        assert!((x[0] - std::f64::consts::SQRT_2 * 10f64.powf((d - 5.007) / 20.0)).abs() < 1e-12);
        assert!(x.iter().all(|v| v.abs() < 1.0));
    }
    let x = tune_block(-5.0);
    let at_1500 = |n: usize| (2.0 * std::f64::consts::PI * 1500.0 * n as f64 / 48_000.0).cos() * x[0];
    assert!((0..NFFT).all(|n| (x[n] - at_1500(n)).abs() < 1e-12));
}
