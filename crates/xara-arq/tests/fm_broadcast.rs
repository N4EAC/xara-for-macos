mod common;

use common::hex;
use xara_arq::cq::Cq;
use xara_arq::fm::broadcast::{Received, cq_buffer, frame, receive, room};
use xara_arq::kiss::{self, Command};
use xara_phy::fm::level::FmBandwidth;

fn cases() -> Vec<xara_testkit::json::Value> {
    common::json("fm/rec16_lab.json")["cases"].as_array().unwrap().clone()
}

#[test]
fn record16_frames_match_vara_fm() {
    let c = cases();
    let want = |i: usize| hex(c[i]["frame_hex"].as_str().unwrap());
    let k = kiss::encode(Command::Generic, b"Hello").unwrap().bytes;
    assert_eq!(frame(FmBandwidth::Narrow, &k).unwrap(), want(0));
    assert_eq!(frame(FmBandwidth::Narrow, &cq_buffer("N0CALL", None).unwrap()).unwrap(), want(1));
    assert_eq!(frame(FmBandwidth::Narrow, &cq_buffer("N0CALL-5", Some("NARROW")).unwrap()).unwrap(), want(2));
    assert_eq!(receive(&want(0)), Some(Received::Kiss(Command::Generic, b"Hello".to_vec())));
    assert_eq!(receive(&want(1)), Some(Received::Cq(Cq { call: "N0CALL".into(), bw: None })));
    assert_eq!(receive(&want(2)), Some(Received::Cq(Cq { call: "N0CALL-5".into(), bw: None })));
    let mut bad = want(1);
    bad[3] ^= 1;
    assert_eq!(receive(&bad), None);
}

#[test]
fn record16_room() {
    assert_eq!(room(FmBandwidth::Narrow), 106);
    assert!(frame(FmBandwidth::Narrow, &[0x21; 107]).is_none());
    assert!(cq_buffer("TOOLONG1", None).is_none());
    assert!(cq_buffer("N0CALL", Some("bad word")).is_none());
}

#[test]
fn record16_fragments_match_vara_fm() {
    let c = cases();
    let v = &c[3];
    let data = hex(v["kiss_data_hex"].as_str().unwrap());
    let want: Vec<Vec<u8>> = v["frames_hex"].as_array().unwrap().iter().map(|h| hex(h.as_str().unwrap())).collect();
    let buf = kiss::encode(Command::Generic, &data).unwrap().bytes;
    assert_eq!(xara_arq::fm::broadcast::frames(FmBandwidth::Narrow, &buf).unwrap(), want);
    let mut r = xara_arq::fm::broadcast::Reassembler::default();
    assert_eq!(r.push(&want[0]), None);
    assert_eq!(r.push(&want[1]), None);
    assert_eq!(r.push(&want[2]), Some(Received::Kiss(Command::Generic, data.clone())));
    let mut r = xara_arq::fm::broadcast::Reassembler::default();
    assert_eq!(r.push(&want[0]), None);
    assert_eq!(r.push(&want[2]), None);
    assert_eq!(r.push(&want[0]), None);
    assert_eq!(r.push(&want[1]), None);
    assert_eq!(r.push(&want[2]), Some(Received::Kiss(Command::Generic, data)));
}

fn record16_frames(bw: FmBandwidth, buf: &[u8]) -> Vec<Vec<u8>> {
    kiss::split_masked(buf, 2, room(bw), kiss::FM_CRC_MASK).iter().map(|f| frame(bw, f).unwrap()).collect()
}

#[test]
fn record16_reassembly_is_bounded_by_5000_bytes() {
    let bw = FmBandwidth::Narrow;
    let mut r = xara_arq::fm::broadcast::Reassembler::default();
    let data = vec![0x80; 2500];
    let long = kiss::encode(Command::Generic, &data).unwrap().bytes;
    let frames = record16_frames(bw, &long);
    assert!(frames.len() > kiss::MAX_FRAGMENTS + 1);
    let got: Vec<Option<Received>> = frames.iter().map(|f| r.push(f)).collect();
    assert_eq!(got.last(), Some(&Some(Received::Kiss(Command::Generic, data))));
    let data = vec![0x80; kiss::RX_BUFFER - 1];
    let fits = kiss::encode(Command::Generic, &data).unwrap().bytes;
    let got: Vec<Option<Received>> = record16_frames(bw, &fits).iter().map(|f| r.push(f)).collect();
    assert_eq!(got.last(), Some(&Some(Received::Kiss(Command::Generic, data))));
    let over = kiss::encode(Command::Generic, &[0x80; kiss::RX_BUFFER + 300]).unwrap().bytes;
    assert!(record16_frames(bw, &over).iter().all(|f| r.push(f).is_none()));
    let data = vec![0x80; 300];
    let buf = kiss::encode(Command::Generic, &data).unwrap().bytes;
    let ok = xara_arq::fm::broadcast::frames(bw, &buf).unwrap();
    let got: Vec<Option<Received>> = ok.iter().map(|f| r.push(f)).collect();
    assert_eq!(got.last(), Some(&Some(Received::Kiss(Command::Generic, data))));
}

#[test]
fn record16_sends_at_most_ten_fragments() {
    let bw = FmBandwidth::Narrow;
    let max = 1 + (room(bw) - 2) + (kiss::FM_MAX_FRAGMENTS - 1) * (room(bw) - 1);
    let buf = |n: usize| kiss::encode(Command::Generic, &vec![0x80; n - 1]).unwrap().bytes;
    let frames = xara_arq::fm::broadcast::frames(bw, &buf(max)).unwrap();
    assert_eq!(frames.len(), kiss::FM_MAX_FRAGMENTS);
    assert!(xara_arq::fm::broadcast::frames(bw, &buf(max + 1)).is_none());
}

#[test]
fn record16_fragments_carry_a_five_bit_crc() {
    let bw = FmBandwidth::Narrow;
    let data = (0u8..=255).map(|b| vec![b; 300]).find(|d| {
        let buf = kiss::encode(Command::Generic, d).unwrap().bytes;
        kiss::crc_bits(&buf, kiss::FM_CRC_MASK) & 0x08 != 0
    });
    let data = data.unwrap();
    let buf = kiss::encode(Command::Generic, &data).unwrap().bytes;
    let frames = xara_arq::fm::broadcast::frames(bw, &buf).unwrap();
    let mut r = xara_arq::fm::broadcast::Reassembler::default();
    let got: Vec<Option<Received>> = frames.iter().map(|f| r.push(f)).collect();
    assert_eq!(got.last(), Some(&Some(Received::Kiss(Command::Generic, data))));
    let nibble: Vec<Vec<u8>> =
        kiss::split(&buf, frames.len(), room(bw)).iter().map(|f| frame(bw, f).unwrap()).collect();
    let mut r = xara_arq::fm::broadcast::Reassembler::default();
    assert!(nibble.iter().all(|f| r.push(f).is_none()));
}

#[test]
fn record16_counts_above_nine_spill_into_the_crc_bits() {
    let bw = FmBandwidth::Narrow;
    let mut seen = [false; 2];
    for b in 0x80u8..=0xFF {
        let data = vec![b; 1000];
        let buf = kiss::encode(Command::Generic, &data).unwrap().bytes;
        let frames = xara_arq::fm::broadcast::frames(bw, &buf).unwrap();
        assert_eq!(frames.len(), kiss::FM_MAX_FRAGMENTS);
        let spill = (frames.len() - 2) as u8 & !kiss::crc_bits(&buf, kiss::FM_CRC_MASK) & kiss::FM_CRC_MASK;
        let mut r = xara_arq::fm::broadcast::Reassembler::default();
        let got = frames.iter().map(|f| r.push(f)).last().unwrap();
        assert_eq!(got.is_some(), spill == 0);
        seen[usize::from(spill == 0)] = true;
    }
    assert_eq!(seen, [true, true]);
}
