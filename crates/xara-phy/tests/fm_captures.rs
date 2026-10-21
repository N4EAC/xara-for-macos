mod common;
use xara_testkit::hex;

use xara_phy::call::call_hash;
use xara_phy::fm::ctrl::{Codebook, Side};
use xara_phy::fm::level::{FmBandwidth, Record};
use xara_phy::fm::rx::receiver::{Event, Receiver};
use xara_phy::fm::tx::{Modulator, TxOptions};

const SKIP: usize = 64;

fn fit(x: &[f64], y: &[f64], at: i64) -> (f64, f64) {
    let mut best = (f64::INFINITY, 0.0);
    for d in -96..=96 {
        let s = at + d;
        if s < 0 || s as usize + y.len() > x.len() {
            continue;
        }
        let seg = &x[s as usize + SKIP..s as usize + y.len()];
        let y = &y[SKIP..];
        let g = seg.iter().zip(y).map(|(a, b)| a * b).sum::<f64>() / y.iter().map(|b| b * b).sum::<f64>();
        let e = seg.iter().zip(y).map(|(a, b)| (a - g * b).powi(2)).sum::<f64>();
        let res = (e / seg.iter().map(|a| a * a).sum::<f64>()).sqrt();
        if res < best.0 {
            best = (res, g);
        }
    }
    best
}

#[test]
#[ignore = "needs the VARA FM lab captures in XARA_CAPTURES; run with --release --ignored"]
fn vara_fm_captures() {
    let refs = common::load("fm/xara_fm_capture_refs");
    let dir = common::captures_dir();
    let key = call_hash(b"N1CALL");
    let cb = Codebook::for_link(b"N0CALL", b"N1CALL");
    let codes: Vec<u32> = (1..=24).collect();
    let mods = [Modulator::new(FmBandwidth::Narrow), Modulator::new(FmBandwidth::Wide)];
    let mut dirs = Vec::new();
    for (run, r) in refs.as_object().unwrap() {
        let bw = if r["mode"] == "WIDE" { FmBandwidth::Wide } else { FmBandwidth::Narrow };
        for (d, c) in r["dirs"].as_object().unwrap() {
            if std::env::var("FM_RUN").map_or(true, |v| format!("{run} {d}").contains(&v)) {
                dirs.push((run, d, bw, c));
            }
        }
    }
    let results = common::par_map(&dirs, |&(run, d, bw, c)| {
        let m = &mods[usize::from(bw == FmBandwidth::Wide)];
        let mut out = DirResult::default();
        let path = dir.join(c["wav"].as_str().unwrap());
        if !path.exists() {
            out.bad.push(format!("{run} {d}: missing {}", path.display()));
            return out;
        }
        let x = common::read_wav(&path);
        let t0 = std::time::Instant::now();
        let mut rx = Receiver::new(bw, key, &[0, 1, 2, 14, 15, 16].map(|i| Record::from_index(i).unwrap()), 1);
        rx.set_ctrl(Side::Source, Some(cb.clone()), &codes);
        let mut ev: Vec<Event> = x.chunks(480).flat_map(|b| rx.feed(b)).collect();
        ev.extend(rx.flush());
        let cpu = t0.elapsed().as_secs_f64() / (x.len() as f64 / 48000.0);
        let txs: Vec<_> = ev.iter().filter_map(|e| if let Event::Tx(t) = e { Some(t) } else { None }).collect();
        let got: Vec<(u64, usize, Vec<String>)> = txs
            .iter()
            .map(|t| {
                (t.record.index() as u64, t.frames.len(), t.frames.iter().map(|f| hex::encode(&f.bytes)).collect())
            })
            .collect();
        let want: Vec<(u64, usize, Vec<String>)> = c["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                let h: Vec<String> = e["hex"].as_array().unwrap().iter().map(|s| s.as_str().unwrap().into()).collect();
                (e["record"].as_u64().unwrap(), h.len(), h)
            })
            .collect();
        if got != want || txs.iter().any(|t| !t.complete) {
            out.bad.push(format!("{run} {d}: transmissions {:?} want {:?}", summary(&got), summary(&want)));
        }
        let ctrl: Vec<(u32, u8)> = ev
            .iter()
            .filter_map(|e| if let Event::Ctrl(k) = e { Some((k.det.code, k.det.lead_row)) } else { None })
            .collect();
        let want_ctrl: Vec<(u32, u8, bool)> = c["ctrl"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                (e["code"].as_u64().unwrap() as u32, e["lead_row"].as_u64().unwrap() as u8, e["detect_only"] == true)
            })
            .collect();
        match match_ctrl(&ctrl, &want_ctrl) {
            Some(m) => out.lead_mismatch += m,
            None => out.bad.push(format!(
                "{run} {d}: codes {:?} want {:?}",
                ctrl.iter().map(|k| k.0).collect::<Vec<_>>(),
                want_ctrl.iter().map(|k| k.0).collect::<Vec<_>>()
            )),
        }
        for t in &txs {
            let frames: Vec<&[u8]> = t.frames.iter().map(|f| f.bytes.as_slice()).collect();
            let y =
                m.transmission(t.record, key, &frames, 0, TxOptions { drive_db: -5.0, pre_signal: false }, &mut |_| 0);
            let at = 8 * (t.frames[0].t - 10);
            let (res, g) = fit(&x, &y, at);
            out.worst = out.worst.max(res);
            if res > 3e-4 || (20.0 * (g * 32768.0 / 32767.0).log10()).abs() > 0.01 {
                out.bad.push(format!("{run} {d}: rec {} fit residual {res:.2e} gain {g:.6}", t.record));
            }
        }
        out.n_tx = txs.len();
        out.n_frames = txs.iter().map(|t| t.frames.len()).sum::<usize>();
        out.n_ctrl = ctrl.len();
        out.line = format!("{run:16} {d}: tx {} ctrl {} cpu {cpu:.3}x", txs.len(), ctrl.len());
        out
    });
    let mut bad = Vec::new();
    let (mut n_tx, mut n_frames, mut n_ctrl, mut worst, mut lead_mismatch) = (0, 0, 0, 0.0f64, 0);
    for r in results {
        if !r.line.is_empty() {
            eprintln!("{}", r.line);
        }
        bad.extend(r.bad);
        n_tx += r.n_tx;
        n_frames += r.n_frames;
        n_ctrl += r.n_ctrl;
        worst = worst.max(r.worst);
        lead_mismatch += r.lead_mismatch;
    }
    eprintln!(
        "{n_tx} transmissions, {n_frames} frames, {n_ctrl} control frames, worst fit residual {worst:.2e}, lead-in report mismatches {lead_mismatch}"
    );
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    assert_eq!((n_tx, n_frames), (99, 110));
}

#[derive(Default)]
struct DirResult {
    bad: Vec<String>,
    n_tx: usize,
    n_frames: usize,
    n_ctrl: usize,
    worst: f64,
    lead_mismatch: usize,
    line: String,
}

fn summary(v: &[(u64, usize, Vec<String>)]) -> Vec<(u64, usize)> {
    v.iter().map(|t| (t.0, t.1)).collect()
}

fn match_ctrl(got: &[(u32, u8)], want: &[(u32, u8, bool)]) -> Option<usize> {
    let Some((&(code, row, loose), rest)) = want.split_first() else { return got.is_empty().then_some(0) };
    if got.first()?.0 != code {
        return None;
    }
    let lead = usize::from(got[0].1 != row && !loose);
    let max = if loose { got.iter().take_while(|g| g.0 == code).count() } else { 1 };
    (1..=max).find_map(|n| match_ctrl(&got[n..], rest)).map(|m| m + lead)
}
