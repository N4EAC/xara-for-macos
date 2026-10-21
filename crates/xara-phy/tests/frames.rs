mod common;
use common::*;
use std::collections::HashMap;
use xara_phy::dat::{RecordTables, hf_tables};
use xara_phy::fec::{frame_bits, interleave, turbo_encode, whiten};
use xara_phy::hf::frame::{Control, build_frame, crc_ok, header_payload, term_marker};
use xara_phy::hf::level::{Bandwidth, Record};
use xara_phy::seq;
use xara_testkit::hex;
use xara_testkit::json::Value;

fn bw_of(v: &Value) -> Bandwidth {
    let hz = v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap();
    Bandwidth::ALL.into_iter().find(|b| u64::from(b.hz()) == hz).unwrap()
}

fn num(v: &Value) -> usize {
    v.as_u64().map(|x| x as usize).unwrap_or_else(|| v.as_str().unwrap().parse().unwrap())
}

fn packhex(bits: &[u8]) -> String {
    hex::encode(
        bits.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |a, (i, &b)| a | (b << (7 - i)))).collect::<Vec<_>>(),
    )
}

struct Chain {
    tables: HashMap<Bandwidth, Vec<RecordTables>>,
    white: Vec<u8>,
}

impl Chain {
    fn new() -> Self {
        Self {
            tables: Bandwidth::ALL.into_iter().map(|b| (b, hf_tables(b))).collect(),
            white: seq::whitening(seq::HF_SEED),
        }
    }

    fn encode(&self, bw: Bandwidth, record: Record, frame: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let l = bw.level(record);
        let t = &self.tables[&bw][record.index()];
        let mut info = frame_bits(frame, l.k as usize);
        whiten(&mut info, &self.white);
        let coded = turbo_encode(&info, &t.turbo, l.puncture_period(), l.n() as usize);
        let tx = interleave(&coded, &t.chan);
        (info, coded, tx)
    }
}

fn check_chain(chain: &Chain, f: &Value, what: &str) {
    let bw = bw_of(&f["mode"]);
    let record = Record::new(num(&f["level"])).unwrap();
    let frame = hex::decode(f["frame_hex"].as_str().unwrap()).unwrap();
    assert!(crc_ok(&frame), "{what}");
    assert_eq!(frame.len(), bw.level(record).frame_bytes() as usize, "{what}");
    let (info, coded, tx) = chain.encode(bw, record, &frame);
    assert_eq!(packhex(&info), f["info_whitened_hex"].as_str().unwrap(), "{what} info");
    assert_eq!(packhex(&coded), f["coded_hex"].as_str().unwrap(), "{what} coded");
    assert_eq!(packhex(&tx), f["tx_hex"].as_str().unwrap(), "{what} tx");
}

#[test]
fn harness_frames() {
    let chain = Chain::new();
    let marker = term_marker(b"N0CALL");
    for name in ["frames_500", "frames_2300", "frames_2750"] {
        for f in load(name)["frames"].as_array().unwrap() {
            let bw = bw_of(&f["mode"]);
            let record = Record::new(num(&f["level"])).unwrap();
            let len = bw.level(record).frame_bytes() as usize;
            let kind = f["kind"].as_str().unwrap();
            let what = format!("{name} L{record} {kind}");
            let data_ctrl = Control { reply: 2, compressed: false, hint: 6 };
            let built = match kind {
                "data_full" => {
                    let data: Vec<u8> = (0..len - 3).map(|i| ((167 * i + 13) % 256) as u8).collect();
                    build_frame(len, &data, data_ctrl, None, marker)
                }
                "session_header" => build_frame(
                    len,
                    &header_payload(b"N0CALL", 0x80),
                    Control { reply: 2, ..Default::default() },
                    Some(record.get() as u8),
                    marker,
                ),
                "data_short" => build_frame(len, b"HELLO VARA", data_ctrl, None, marker),
                k => panic!("{k}"),
            };
            assert_eq!(hex::encode(&built), f["frame_hex"].as_str().unwrap(), "{what}");
            check_chain(&chain, f, &what);
        }
    }
}

#[test]
fn vara_frames() {
    let chain = Chain::new();
    let frames = load("vara_frames")["frames"].as_array().unwrap().clone();
    assert_eq!(frames.len(), 39);
    for (i, f) in frames.iter().enumerate() {
        check_chain(&chain, f, &format!("vara_frames[{i}] {} L{} {}", f["mode"], f["level"], f["kind"]));
    }
}

#[test]
fn control_byte() {
    for b in [0x99u8, 0x89, 0x59, 0x19, 0x82, 0x4b, 0x0b, 0xa2] {
        let (c, last, full) = Control::from_byte(b);
        assert_eq!(c.byte(last, full), b);
    }
}
