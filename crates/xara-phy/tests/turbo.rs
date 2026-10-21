mod common;

use common::{Rng, random_frame};
use xara_phy::decode::{EXTRINSIC_SCALE, FrameDecoder, ITERATIONS, MIN_ITERATIONS};
use xara_phy::hf::level::{Bandwidth, Record};
use xara_phy::hf::tx::Modulator;
use xara_phy::seq;
use xara_phy::turbo::{CrcCheck, Decoder, Kind, split, used_len};

#[test]
fn noisy_llrs_decode_on_every_level() {
    let mut rng = Rng::new(1);
    for bw in Bandwidth::ALL {
        let m = Modulator::new(bw);
        let d = FrameDecoder::new(bw);
        for rec in bw.levels() {
            let l = bw.level(rec);
            let frame = random_frame(&mut rng, l.frame_bytes() as usize);
            let tx = m.encode(rec, &frame);
            let llr: Vec<f64> = tx.iter().map(|&b| (1.0 - 2.0 * f64::from(b)) * 2.0 + rng.gauss()).collect();
            let out = d.decode(rec, &llr);
            assert!(out.ok, "{bw:?} record {rec}");
            assert_eq!(out.bytes, frame, "{bw:?} record {rec}");
            assert!(out.iterations < 10, "{bw:?} record {rec}: {} iterations", out.iterations);
        }
    }
}

#[test]
fn noise_fails_crc() {
    let mut rng = Rng::new(2);
    let d = FrameDecoder::new(Bandwidth::Hz2300);
    for rec in [1, 5, 12].map(|r| Record::new(r).unwrap()) {
        let n = Bandwidth::Hz2300.level(rec).n() as usize;
        let llr: Vec<f64> = (0..n).map(|_| rng.gauss()).collect();
        let out = d.decode(rec, &llr);
        assert!(!out.ok);
        assert_eq!(out.iterations, 10);
    }
}

#[test]
fn both_kinds_decode_every_level() {
    let mut rng = Rng::new(3);
    let white = seq::whitening(seq::HF_SEED);
    for bw in Bandwidth::ALL {
        let m = Modulator::new(bw);
        let d = FrameDecoder::new(bw);
        for rec in bw.levels() {
            let l = bw.level(rec);
            let t = &d.tables[rec.index()];
            let (k, n, p) = (l.k as usize, l.n() as usize, l.puncture_period() as usize);
            let frame = random_frame(&mut rng, l.frame_bytes() as usize);
            let tx = m.encode(rec, &frame);
            let mut soft = vec![0.0; n];
            for (i, &c) in t.chan.iter().enumerate() {
                soft[c as usize] = (1.0 - 2.0 * f64::from(tx[i])) * 2.0 + rng.gauss();
            }
            let sp = split(&soft[..used_len(k, p)], &t.turbo, p);
            let check = CrcCheck { white: &white, nbytes: k / 8 };
            for kind in [Kind::Float, Kind::Fixed] {
                let out = Decoder::new(kind).decode(
                    &sp,
                    &t.turbo,
                    ITERATIONS,
                    EXTRINSIC_SCALE,
                    Some((MIN_ITERATIONS, &check)),
                );
                assert_eq!(check.bytes(&out.bits), frame, "{kind:?} {bw:?} record {rec}");
                assert!(out.iterations < ITERATIONS, "{kind:?} {bw:?} record {rec}: {} iterations", out.iterations);
            }
        }
    }
}
