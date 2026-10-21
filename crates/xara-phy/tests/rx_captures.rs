mod common;
use xara_testkit::hex;

use std::collections::BTreeSet;
use xara_phy::call::call_hash;
use xara_phy::hf::ctrl::{Codebook, Role};
use xara_phy::hf::level::Record;
use xara_phy::hf::rx::ctrl::rx_entries;
use xara_phy::hf::rx::receiver::{Event, Receiver};

fn strings(v: &xara_testkit::json::Value) -> BTreeSet<String> {
    v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
}

#[test]
#[ignore = "needs the VARA lab captures in XARA_CAPTURES; run with --release --ignored"]
fn vara_captures() {
    let refs = common::load("xara_capture_refs");
    let dir = common::captures_dir();
    let caps: Vec<_> = refs.as_object().unwrap().iter().collect();
    let results = common::par_map(&caps, |&(name, c)| {
        let path = dir.join(c["wav"].as_str().unwrap());
        if !path.exists() {
            return (format!("{name}: missing {}", path.display()), false);
        }
        let bw = common::bw_of(&c["bw"]);
        let levels: Vec<Record> =
            c["levels"].as_array().unwrap().iter().filter_map(|v| Record::new(v.as_u64().unwrap() as usize)).collect();
        let entries = c.get("calls").map(|calls| {
            let my = calls[0].as_str().unwrap().as_bytes();
            let remote = calls[1].as_str().unwrap().as_bytes();
            let mut cb = Codebook::new(call_hash(remote), [call_hash(my), 0, 0, 0, 0], Role::Caller, bw);
            cb.set_connect_codes(call_hash(my), Role::Caller);
            rx_entries(&cb, 0)
        });
        let x = common::read_wav(&path);
        let t0 = std::time::Instant::now();
        let mut rx = Receiver::new(bw, &levels, 1, entries.as_deref());
        let mut ev: Vec<Event> = x.chunks(480).flat_map(|b| rx.feed(b)).collect();
        ev.extend(rx.flush());
        let cpu = t0.elapsed().as_secs_f64() / (x.len() as f64 / 48000.0);
        let ok: BTreeSet<String> = ev
            .iter()
            .filter_map(|e| match e {
                Event::Frame(f) if f.frame.ok => Some(hex::encode(&f.frame.bytes)),
                _ => None,
            })
            .collect();
        let txs: Vec<_> = ev.iter().filter_map(|e| if let Event::Tx(t) = e { Some(t) } else { None }).collect();
        let failed = txs.iter().filter(|t| !t.ok().iter().all(|&o| o)).count();
        let ctrl: Vec<u64> =
            ev.iter().filter_map(|e| if let Event::Ctrl(c) = e { Some(u64::from(c.det.code)) } else { None }).collect();
        let nctrl = ctrl.len();
        let py_codes: Vec<u64> = c
            .get("rt_rx_ctrl")
            .map_or(vec![], |v| v.as_array().unwrap().iter().map(|e| e[0].as_u64().unwrap()).collect());
        let py_tx = c["rt_rx_tx"].as_array().unwrap().len();
        let (r, py) = (strings(&c["ref"]), strings(&c["rt_rx"]));
        let py_ctrl = c.get("rt_rx_ctrl").map_or(0, |v| v.as_array().unwrap().len());
        let line = format!(
            "{name:22} ref {}/{} rt_rx {}/{} decoded {} tx {} (py {py_tx}) failed {failed} ctrl {nctrl} (py {py_ctrl}) cpu {cpu:.3}x",
            r.intersection(&ok).count(),
            r.len(),
            py.intersection(&ok).count(),
            py.len(),
            ok.len(),
            txs.len(),
        );
        let bad = !r.is_subset(&ok) || !py.is_subset(&ok) || failed > 0 || txs.len() != py_tx || ctrl != py_codes;
        (line, bad)
    });
    let report: Vec<&str> = results.iter().map(|r| r.0.as_str()).collect();
    let bad: Vec<&str> = results.iter().filter(|r| r.1).map(|r| r.0.as_str()).collect();
    println!("{}", report.join("\n"));
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
