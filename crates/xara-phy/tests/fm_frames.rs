mod common;
use xara_phy::call::call_hash;
use xara_phy::fm::ctrl::{Codebook, LEAD_IN, LeadIn, TAMPER_KEY, codes};
use xara_phy::fm::frame::{Kind, Record0, Trailer, build_data, parse_data, snr_byte};
use xara_phy::fm::level::{FmBandwidth, Record};
use xara_testkit::hex;

fn rec(i: usize) -> Record {
    Record::from_index(i).unwrap()
}

fn rows(s: &str) -> Vec<[u32; 3]> {
    s.split_whitespace()
        .map(|r| {
            let v: Vec<u32> = r.split('+').map(|t| t.parse().unwrap()).collect();
            [v[0], v[1], v[2]]
        })
        .collect()
}

#[test]
fn record0_spec_vector() {
    let f = Record0::connect("N0CALL", "N1CALL", "", "", false, false).to_bytes().unwrap();
    assert_eq!(hex::encode(&f), "004e3043414c4c2000202020202020200020202020202020004e3143414c4c200000080700f8f3");
    let r = Record0 {
        kind: Kind::Ping,
        hop: 1,
        compression: 1,
        aes: true,
        chain_snr: snr_byte(12.3),
        ..Record0::connect("N0CALL-7", "N1CALL-T", "N9DIG", "", true, true)
    };
    assert_eq!(Record0::parse(&r.to_bytes().unwrap()), Some(r));
    assert_eq!(Record0::connect("N0CALL", "TOOLONGCALL", "", "", false, false).to_bytes(), None);
    assert_eq!((snr_byte(-3.0), snr_byte(0.5), snr_byte(40.0)), (1, 4, 245));
}

#[test]
fn fm_lab_frames() {
    let v = common::load_doc("fm/lab_frames");
    for c in v["record0"].as_array().unwrap() {
        let f = hex::decode(c["frame_hex"][0].as_str().unwrap()).unwrap();
        let r = Record0::parse(&f).expect("record 0 CRC");
        let p = &c["record0"];
        assert_eq!(r.source, p["source"].as_str().unwrap());
        assert_eq!(r.target, p["target"].as_str().unwrap());
        assert_eq!(r.via1, p["via1"].as_str().unwrap());
        assert_eq!(u64::from(r.hop), p["hop"].as_u64().unwrap());
        assert_eq!(r.wide, p["wide"].as_u64().unwrap() == 1);
        assert_eq!(r.to_bytes().unwrap(), f);
    }
    let mut n = 0;
    for c in v["cases"].as_array().unwrap() {
        let record = rec(c["record"].as_u64().unwrap() as usize);
        let bw = if c["mode"] == "WIDE" { FmBandwidth::Wide } else { FmBandwidth::Narrow };
        for (h, fl) in c["frame_hex"].as_array().unwrap().iter().zip(c["fields"].as_array().unwrap()) {
            let f = hex::decode(h.as_str().unwrap()).unwrap();
            assert_eq!(f.len(), bw.level(record).frame_bytes());
            let d = parse_data(record, &f, false).expect("CRC and terminator");
            assert_eq!(d.data.len() as u64, fl["data_len"].as_u64().unwrap());
            assert_eq!(u64::from(d.control.hint), fl["hint"].as_u64().unwrap());
            assert_eq!(d.control.last, fl["last"] == 1);
            assert_eq!(d.control.full, fl["full"] == 1);
            let p = xara_phy::fm::frame::payload_len(record, f.len(), false).unwrap();
            assert_eq!(hex::encode(&f[p + 1..f.len() - 2]), fl["trailer"]);
            let back =
                build_data(record, f.len(), &d.data, d.control.compressed, d.control.hint, d.control.last, d.trailer);
            assert_eq!(back, f);
            n += 1;
        }
    }
    assert_eq!(n, 85);
}

#[test]
fn data_layouts() {
    for record in [1, 2, 14, 15, 16].map(rec) {
        let len = FmBandwidth::Narrow.level(record).frame_bytes();
        for vias in [false, true] {
            if vias && record == Record::Broadcast {
                continue;
            }
            let t = Trailer { lead_report: 27, level: 2, chain_snr: vias.then_some(99) };
            for n in [0, 1, 5, 400] {
                let data: Vec<u8> = (0..n).map(|i| if i % 3 == 0 { 0x14 } else { i as u8 }).collect();
                let f = build_data(record, len, &data, false, 5, true, t);
                assert_eq!(f.len(), len);
                let d = parse_data(record, &f, vias).unwrap();
                let room = xara_phy::fm::frame::payload_len(record, len, vias).unwrap();
                assert_eq!(d.data, data[..n.min(room)]);
                assert_eq!(d.control.full, n >= room);
                if record != Record::Broadcast {
                    assert_eq!(d.trailer, Trailer { level: if record == Record::Start { 2 } else { 0 }, ..t });
                }
            }
        }
    }
}

#[test]
fn ctrl_codebooks() {
    let v = common::load_doc("fm/ctrl_codebooks");
    let c = &v["content"];
    let lead: Vec<[u32; 3]> =
        c["leadin_table"].as_array().unwrap().iter().flat_map(|r| rows(r.as_str().unwrap())).collect();
    assert_eq!(lead, LEAD_IN.to_vec());
    let tamper = codes(TAMPER_KEY);
    for (k, r) in c["tamper_codebook_codes_1_22"].as_object().unwrap() {
        assert_eq!(rows(r.as_str().unwrap()), tamper[k.parse::<usize>().unwrap() - 1].to_vec(), "tamper {k}");
    }
    let mut n = 0;
    for st in c["codebooks"].as_array().unwrap() {
        for (side, info) in st["sides"].as_object().unwrap() {
            let key = |f: &str| u32::from_str_radix(info[f].as_str().unwrap().trim_start_matches("0x"), 16).unwrap();
            let (kd, ku) = (key("key_codes_1_22"), key("key_codes_23_24"));
            assert_eq!(kd, call_hash(info["key_call_1_22"].as_str().unwrap().as_bytes()));
            assert_eq!(ku, call_hash(info["key_call_23_24"].as_str().unwrap().as_bytes()));
            let cb = Codebook::new(kd, ku);
            for (code, r) in st["codes"][side]["fm"].as_object().unwrap() {
                assert_eq!(
                    rows(r.as_str().unwrap()),
                    cb.code(code.parse().unwrap()).to_vec(),
                    "{} {side} {code}",
                    st["station"]
                );
                n += 1;
            }
        }
    }
    assert_eq!(n, 10 * 24);
}

#[test]
fn ctrl_on_air() {
    let v = common::load_doc("fm/lab_ctrl");
    let cb = Codebook::for_link(b"N0CALL", b"N1CALL");
    let mut n = 0;
    for (code, e) in v["codes"].as_object().unwrap() {
        if let Some(r) = e["rows_codebook"].as_str() {
            assert_eq!(rows(r), cb.code(code.parse().unwrap()).to_vec(), "code {code}");
            n += 1;
        }
    }
    assert_eq!(n, 14);
    for (i, r) in v["code24_wide"]["rows"].as_array().unwrap().iter().enumerate() {
        let mut want: Vec<u32> = r.as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect();
        let mut got = cb.code(24)[i].to_vec();
        want.sort_unstable();
        got.sort_unstable();
        assert_eq!(got, want, "code 24 row {}", i + 2);
    }
}

#[test]
fn lead_in_adaptation() {
    let mut l = LeadIn::default();
    assert_eq!((l.report(8), l.report(29 - 11 + 7), l.report(29 - 8 + 7)), (11, 8, 8));
    let mut l = LeadIn::default();
    assert_eq!((l.report(1), l.data_symbols()), (1, 1));
    let mut l = LeadIn::default();
    assert_eq!(l.report(2), 2);
    assert_eq!(LeadIn::default().data_symbols(), 25);
    let mut l = LeadIn::default();
    for r in [1, 1, 1] {
        l.report(r);
    }
    assert_eq!(l.report(29), 2);
    assert_eq!(l.report(29), 2);
}

#[test]
fn record0_relay_keeps_received_compression_and_aes_bits() {
    let mut raw = Record0::connect("N0CALL", "N1CALL", "N9DIG", "", false, false).to_bytes().unwrap();
    raw[34] = 0xC0 | 0x08 | 0x04 | 0x31;
    let n = raw.len() - 2;
    let crc = xara_phy::crc::crc16_genibus(&raw[..n]);
    raw[n..].copy_from_slice(&crc.to_be_bytes());
    let r = Record0::parse(&raw).unwrap();
    assert_eq!((r.registered, r.compression), (false, 2));
    assert_eq!(r.to_bytes().unwrap()[34], 0x48);
    assert_eq!(r.restamp(&raw)[34], 0xCC);
}
