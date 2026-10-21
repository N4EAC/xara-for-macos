mod common;
use common::*;
use xara_phy::call::{call_hash, pack_call};
use xara_phy::crc::crc16_genibus;
use xara_testkit::hex;

fn latin1(s: &str) -> Vec<u8> {
    s.chars().map(|c| u8::try_from(u32::from(c)).expect("latin-1")).collect()
}

#[test]
fn crc() {
    let c = load("crc_hash_pack");
    for (name, v) in c["crc16_genibus"].as_object().unwrap() {
        let data = hex::decode(v["hex"].as_str().unwrap()).unwrap();
        assert_eq!(u32::from(crc16_genibus(&data)), hex_u32(&v["crc"]), "{name}");
    }
}

#[test]
fn hash() {
    let c = load("crc_hash_pack");
    for (call, v) in c["call_hash"].as_object().unwrap() {
        let b = latin1(call);
        assert_eq!(u32::from(crc16_genibus(&b)), hex_u32(&v["crc_call"]), "{call}");
        assert_eq!(call_hash(&b), hex_u32(&v["hash"]), "{call}");
    }
}

#[test]
fn pack() {
    let c = load("crc_hash_pack");
    for (call, v) in c["packed_call"].as_object().unwrap() {
        assert_eq!(hex::encode(pack_call(&latin1(call))), v["hex"].as_str().unwrap(), "{call}");
    }
}

#[test]
fn hash_and_packing_items() {
    let c = load("alphabet_table");
    for (call, h) in c["call_hash"].as_object().unwrap() {
        assert_eq!(call_hash(call.as_bytes()), hex_u32(h), "{call}");
    }
    for (call, p) in c["packed_call"].as_object().unwrap() {
        assert_eq!(hex::encode(pack_call(call.as_bytes())), p.as_str().unwrap(), "{call}");
    }
    let hdr = hex::decode(c["session_header_L4_2300"].as_str().unwrap()).unwrap();
    let (body, crc) = hdr.split_at(hdr.len() - 2);
    assert_eq!(crc16_genibus(body).to_be_bytes(), crc);
}
