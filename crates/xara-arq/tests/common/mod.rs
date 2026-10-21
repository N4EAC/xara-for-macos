#![allow(dead_code)]

use xara_phy::hf::level::{Bandwidth, Record};
use xara_testkit::json::Value;

pub fn json(rel: &str) -> Value {
    let p = format!("{}/../../vectors/{rel}", env!("CARGO_MANIFEST_DIR"));
    xara_testkit::json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"))).unwrap()
}

pub fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

pub fn hexv(v: &Value) -> Vec<u8> {
    hex(v.as_str().unwrap())
}

pub fn rec(n: usize) -> Record {
    Record::new(n).unwrap()
}

pub fn recv(v: &Value) -> Record {
    rec(v.as_u64().unwrap() as usize)
}

pub fn bw(v: &Value) -> Bandwidth {
    v.to_string().parse().unwrap()
}
