mod common;

use common::hex;
use xara_arq::textcomp::{compress, decompress};

fn pack(bits: &[bool]) -> Vec<u8> {
    let mut b = bits.to_vec();
    let p = !*b.last().unwrap();
    b.extend(std::iter::repeat_n(p, 8 - b.len() % 8));
    b.chunks(8).map(|c| c.iter().fold(0u8, |a, &x| a << 1 | u8::from(x))).collect()
}

fn vara_expansion(state: u64, w: &str) -> Vec<u8> {
    let mut bits: Vec<bool> = (0..3).rev().map(|i| (state - 1) >> i & 1 == 1).collect();
    bits.extend(w.bytes().map(|c| c == b'1'));
    (4..=bits.len()).rev().find_map(|n| decompress(&pack(&bits[..n]))).map(|(t, _)| t).unwrap_or_default()
}

fn stream(frame: &[u8]) -> (bool, Vec<u8>) {
    let ctrl = frame[frame.len() - 3];
    let mut field = frame[..frame.len() - if ctrl & 2 != 0 { 4 } else { 3 }].to_vec();
    if ctrl & 1 == 0 {
        field.truncate(field.iter().rposition(|&b| b == 0x14).unwrap());
    }
    (ctrl & 0x20 != 0, field)
}

#[test]
fn decodes_like_vara() {
    let v = common::json("textcomp_blackbox.json");
    let fps: Vec<&str> = v["fingerprints"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    let mut n = 0;
    for p in v["decode"].as_array().unwrap() {
        let w = format!("{}{}", p["u"].as_str().unwrap(), fps[p["f"].as_u64().unwrap() as usize]);
        assert_eq!(vara_expansion(p["s"].as_u64().unwrap(), &w), hex(p["out"].as_str().unwrap()), "{p}");
        n += 1;
    }
    for p in v["check"].as_array().unwrap() {
        let s = p["s"].as_u64().unwrap();
        assert_eq!(vara_expansion(s, p["w"].as_str().unwrap()), hex(p["out"].as_str().unwrap()), "{p}");
        n += 1;
    }
    assert!(n >= 1300, "{n}");
}

#[test]
fn encodes_like_vara() {
    let v = common::json("textcomp_blackbox.json");
    let mut n = 0;
    for e in v["encode"].as_array().unwrap() {
        let text = hex(e["text"].as_str().unwrap());
        let mut pos = 0;
        for f in e["frames"].as_array().unwrap() {
            let (comp, data) = stream(&hex(f.as_str().unwrap()));
            if !comp {
                assert_eq!(text[pos..pos + data.len()], data);
                pos += data.len();
                continue;
            }
            let start = usize::from(data[0] >> 5) + 1;
            let end = text.len().min(pos + 3 * 89 + 39);
            let c = compress(&text[pos..end], 89, start);
            assert_eq!(c.bytes, data, "text at {pos}");
            assert_eq!(decompress(&c.bytes).unwrap().0, text[pos..pos + c.consumed]);
            pos += c.consumed;
            n += 1;
        }
        assert_eq!(pos, text.len());
    }
    assert!(n >= 55, "{n}");
}

#[test]
fn round_trip() {
    let v = common::json("textcomp_blackbox.json");
    for e in v["encode"].as_array().unwrap() {
        let text = hex(e["text"].as_str().unwrap());
        for start in 1..=8 {
            for cap in [8, 20, 47, 88, 89, 400] {
                let c = compress(&text, cap, start);
                assert!(c.bytes.len() <= cap, "cap {cap}");
                let (back, _) = decompress(&c.bytes).unwrap();
                assert_eq!(back, text[..c.consumed], "start {start} cap {cap}");
            }
        }
    }
}

#[test]
fn decodes_vara_on_air_frames() {
    let v = common::json("textcomp_frames.json");
    let mut n = 0;
    for f in v["onair"].as_array().unwrap().iter().filter(|f| f["b5"] == 1) {
        let (text, _) = decompress(&hex(f["data_hex"].as_str().unwrap())).unwrap();
        assert_eq!(text, hex(f["text_hex"].as_str().unwrap()), "{} t={}", f["run"], f["t"]);
        n += 1;
    }
    assert_eq!(n, 15);
}
