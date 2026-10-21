mod common;

use common::hexv;
use xara_arq::ax25::{self, AprsType};
use xara_arq::cq;
use xara_arq::hf::frames::queue_hint;
use xara_arq::kiss::{self, BW, Buffer, Command, Push, Reassembler};
use xara_host::kiss::{Deframer, escape};
use xara_phy::hf::frame::{Control, crc_ok};
use xara_phy::hf::level::{Bandwidth, Record};
use xara_phy::hf::tx::Modulator;
use xara_testkit::json::Value;

const L4: Record = Record::new(4).unwrap();

fn num(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}

fn coded_hex(m: &Modulator, level: Record, frame: &[u8]) -> (Vec<u8>, usize) {
    let bits = m.encode(level, frame);
    let packed = bits.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |a, (i, &b)| a | (b << (7 - i)))).collect();
    (packed, bits.len())
}

struct Sent {
    level: Record,
    buf: Buffer,
    frames: Vec<Vec<u8>>,
}

fn send(kiss_in: &[u8]) -> Sent {
    let got = Deframer::default().feed(kiss_in);
    assert_eq!(got.len(), 1);
    let (cmd, data) = &got[0];
    let buf = kiss::encode(Command::from_byte(*cmd).unwrap(), data).unwrap();
    let (level, fields) = kiss::fragments(&buf).unwrap();
    let ctrl = Control { reply: 0, compressed: false, hint: queue_hint(BW, level, 0) };
    let frames = fields.iter().map(|f| kiss::frame(f, level, ctrl, 0)).collect();
    Sent { level, buf, frames }
}

fn receive(level: Record, frames: &[Vec<u8>]) -> Vec<u8> {
    let mut r = Reassembler::default();
    let mut out = None;
    for (k, f) in frames.iter().enumerate() {
        assert!(crc_ok(f));
        match r.push(f, level) {
            Push::More => assert!(k + 1 < frames.len()),
            Push::Done(b) => out = Some(b),
            Push::Dropped => panic!("fragment {k} dropped"),
        }
    }
    let (cmd, data) = kiss::decode(&out.unwrap()).unwrap();
    escape(cmd.byte(), &data)
}

fn check_case(c: &Value, m: &Modulator) -> Sent {
    let name = c["name"].as_str().unwrap();
    let s = send(&hexv(&c["kiss_tcp_in_hex"]));
    assert_eq!(s.buf.bytes, hexv(&c["vara_kiss_buffer_hex"]), "{name}");
    assert_eq!(s.level.get(), num(&c["level"]), "{name}");
    assert_eq!(kiss::code_for_level(s.level), c["announce_code"].as_u64().unwrap() as u32, "{name}");
    let want = c["frames"].as_array().unwrap();
    assert_eq!(s.frames.len(), want.len(), "{name}");
    assert_eq!(s.frames.len(), num(&c["fragments"]), "{name}");
    for (f, w) in s.frames.iter().zip(want) {
        assert_eq!(f, &hexv(&w["frame_hex"]), "{name} frame {}", w["index"]);
        let field = hexv(&w["field_hex"]);
        assert_eq!(f[..field.len()], field[..], "{name}");
        assert_eq!(u64::from(f[f.len() - 3]), w["control_byte"].as_u64().unwrap(), "{name}");
        let (bits, n) = coded_hex(m, s.level, f);
        assert_eq!(n, num(&w["coded_tx_bits"]), "{name}");
        assert_eq!(bits, hexv(&w["coded_tx_bits_hex"]), "{name}");
    }
    assert_eq!(receive(s.level, &s.frames), hexv(&c["receiver_kiss_tcp_out_hex"]), "{name}");
    s
}

#[test]
fn generic_cases() {
    let m = Modulator::new(BW);
    let cases = common::json("kiss/generic.json")["cases"].as_array().unwrap().clone();
    for c in &cases {
        check_case(c, &m);
        assert_eq!(kiss::generic_encode(&hexv(&c["data_hex"])), hexv(&c["vara_kiss_buffer_hex"]));
    }
    assert_eq!(cases.len(), 5);
}

#[test]
fn ax25_cases() {
    let m = Modulator::new(BW);
    let cases = common::json("kiss/ax25.json")["cases"].as_array().unwrap().clone();
    for c in &cases {
        let name = c["name"].as_str().unwrap();
        let s = check_case(c, &m);
        let frame = hexv(&c["ax25_frame_hex"]);
        let cmd = Command::from_byte(c["kiss_command"].as_u64().unwrap() as u8).unwrap();
        let rec = ax25::parse(&frame, cmd).unwrap();
        let r = &c["record"];
        assert_eq!(rec.dest, r["dest"].as_str().unwrap(), "{name}");
        assert_eq!(rec.src, r["src"].as_str().unwrap(), "{name}");
        let digis: Vec<&str> = r["digis"].as_array().unwrap().iter().map(|d| d.as_str().unwrap()).collect();
        assert_eq!(rec.digis, digis, "{name}");
        assert_eq!(u64::from(rec.aprs_type.code()), r["aprs_type"].as_u64().unwrap(), "{name}");
        assert_eq!(rec.alias as u64, r["alias_index"].as_u64().unwrap(), "{name}");
        assert_eq!(u64::from(s.buf.aprs_type.code()), r["aprs_type"].as_u64().unwrap(), "{name}");
        let bits: String = c["bit_fields"].as_array().unwrap().iter().map(|f| f["bits"].as_str().unwrap()).collect();
        let packed: Vec<u8> =
            (0..bits.len()).step_by(8).map(|i| u8::from_str_radix(&bits[i..i + 8], 2).unwrap()).collect();
        assert_eq!(s.buf.bytes[1..], packed[..], "{name}");
        let (_, out) = ax25::decode(&s.buf.bytes).unwrap();
        assert_eq!(out, hexv(&c["receiver_ax25_frame_hex"]), "{name}");
        assert_eq!(out == frame, c["receiver_equals_input"].as_bool().unwrap(), "{name}");
    }
    assert_eq!(cases.len(), 12);
}

#[test]
fn lab_kiss_frames() {
    let lab = common::json("kiss/vara_lab.json");
    let cases = lab["cases"].as_array().unwrap();
    for c in cases {
        let name = c["name"].as_str().unwrap();
        assert!(c["ok"].as_bool().unwrap(), "{name}");
        let o = &c["observed"];
        let s = send(&hexv(&c["kiss_tcp_in_hex"]));
        assert_eq!(kiss::code_for_level(s.level), o["announce_code"].as_u64().unwrap() as u32, "{name}");
        let want: Vec<Vec<u8>> = o["frames_hex"].as_array().unwrap().iter().map(hexv).collect();
        assert_eq!(s.frames, want, "{name}");
        assert_eq!(receive(s.level, &want), hexv(&o["receiver_kiss_tcp_out_hex"]), "{name}");
    }
    assert_eq!(cases.len(), 17);
    let z = &lab["extra_z_timestamp"];
    let s = send(&hexv(&z["kiss_tcp_in_hex"]));
    let want: Vec<Vec<u8>> = z["observed"]["frames_hex"].as_array().unwrap().iter().map(hexv).collect();
    assert_eq!(s.frames, want);
    assert_eq!(s.buf.aprs_type, AprsType::Text);
    assert_eq!(receive(s.level, &want), hexv(&z["observed"]["receiver_kiss_tcp_out_hex"]));
}

#[test]
fn cq_cases() {
    let m = Modulator::new(BW);
    let v = common::json("kiss/cqframe.json");
    for c in v["cases"].as_array().unwrap() {
        let cmd = c["host_cmd"].as_str().unwrap();
        let args: Vec<&str> = cmd.split(' ').collect();
        let f = cq::frame(args[1], args[2].parse().unwrap_or(Bandwidth::Hz2750)).unwrap();
        assert_eq!(f[..6], hexv(&c["payload_hex"])[..], "{cmd}");
        assert_eq!(f, hexv(&c["frame_hex"]), "{cmd}");
        assert_eq!(f.len(), kiss::frame_len(cq::RECORD));
        let (bits, n) = coded_hex(&m, cq::RECORD, &f);
        assert_eq!((bits, n), (hexv(&c["coded_tx_bits_hex"]), num(&c["coded_tx_bits"])), "{cmd}");
        assert_eq!(cq::parse(&f).unwrap().host_line() + "\r", c["receiver_host_line"].as_str().unwrap(), "{cmd}");
    }
    let lab = common::json("kiss/xara_lab.json");
    let cases = lab["cq"].as_array().unwrap();
    for c in cases {
        let cmd = c["host_cmd"].as_str().unwrap();
        let args: Vec<&str> = cmd.split(' ').collect();
        let f = cq::frame(args[1], args[2].parse().unwrap_or(Bandwidth::Hz2750)).unwrap();
        assert_eq!(f, hexv(&c["frame_hex"]), "{cmd}");
        let lines: Vec<&str> = c["receiver_lines"].as_array().unwrap().iter().map(|l| l.as_str().unwrap()).collect();
        if c["receiver_chat"].as_bool() != Some(false) {
            assert_eq!(lines.last().copied(), Some(cq::parse(&f).unwrap().host_line().as_str()), "{cmd}");
            assert_eq!(lines.last().copied(), Some(cmd), "{cmd}");
        }
    }
    assert_eq!(cases.len(), 6);
    let mut p = cq::payload("N0CALL", Bandwidth::Hz500).unwrap();
    p[cq::PAYLOAD - 1] &= !0x30;
    assert_eq!(cq::parse(&p).unwrap().host_line(), "CQFRAME N0CALL ");
}

#[test]
fn deframer_rules() {
    let mut d = Deframer::default();
    assert_eq!(d.feed(&[0xC0, 0xC0, 0x02, 0x41, 0xDB, 0xDC, 0xC0]), vec![(2, vec![0x41, 0xC0])]);
    assert!(d.feed(&[0xC0, 0x01, 0x0A, 0xC0, 0xC0, 0x12, 0x41, 0xC0, 0xC0, 0x06, 0x41, 0xC0]).is_empty());
    let short_ax25 = [vec![0xC0, 0x00], vec![0x40; 15], vec![0xC0]].concat();
    assert!(d.feed(&short_ax25).is_empty());
    let ax25 = [vec![0xC0, 0x00], vec![0x40; 16], vec![0xC0]].concat();
    assert_eq!(d.feed(&ax25).len(), 1);
}

#[test]
fn generic_round_trips() {
    let all: Vec<u8> = (0..=255).collect();
    for data in [&b"A"[..], b"Hello", b"CQ CQ DE!", &[0x14; 30], &all, &[b'x'; 700]] {
        let buf = Buffer { bytes: kiss::generic_encode(data), aprs_type: AprsType::Text };
        let (level, fields) = kiss::fragments(&buf).unwrap();
        let ctrl = Control { reply: 0, compressed: false, hint: queue_hint(BW, level, 0) };
        let frames: Vec<Vec<u8>> = fields.iter().map(|f| kiss::frame(f, level, ctrl, 0x5A)).collect();
        assert_eq!(receive(level, &frames), escape(2, data));
    }
    for short in [&[][..], &[0x21], &[0x20]] {
        assert!(kiss::generic_decode(short).is_empty());
    }
    let big = Buffer { bytes: kiss::generic_encode(&[0x80; 800]), aprs_type: AprsType::Text };
    assert_eq!(kiss::fragments(&big), Err(kiss::Error::TooLong));
}

#[test]
fn split_is_total() {
    assert!(kiss::split(&[], 1, 9).is_empty());
    assert!(kiss::split(b"abc", 2, 2).is_empty());
    assert!(kiss::split(b"abc", 1, 1).is_empty());
    assert!(kiss::split(b"abc", 3, 0).is_empty());
    assert!(kiss::split(b"a", 2, 1).is_empty());
    assert_eq!(kiss::split(b"a", 1, 1), vec![b"a".to_vec()]);
    assert_eq!(kiss::split(b"abc", 1, 2), vec![vec![b'a' | 0x80, b'b'], b"ac".to_vec()]);
}

#[test]
fn call_tokens() {
    use xara_arq::bits::{Bits, Reader};
    for call in [
        "WIDE1-1", "WIDE2", "PNW2-1", "MW1", "OR2-1", "WA1-1", "ALL", "CQ", "APIN21", "APWW15", "APDR47", "APBPQ1",
        "APJYC1", "APOPYT", "APU25N", "APVARA", "APRS", "APX200", "N0CALL", "K1A-15", "AB1CD-7",
    ] {
        let mut b = Bits::default();
        ax25::token_encode(call, 6, &mut b).unwrap();
        let mut r = Reader::new(&b.0);
        assert_eq!(ax25::token_decode(&mut r, 6).as_deref(), Some(call));
        assert_eq!(r.pos, b.len(), "{call}");
    }
    let mut b = Bits::default();
    ax25::token_encode("WIDE2-2", 6, &mut b).unwrap();
    assert_eq!(ax25::token_decode(&mut Reader::new(&b.0), 6).as_deref(), Some("WIDE2-10"));
    assert!(matches!(ax25::token_encode("N0C@LL", 6, &mut b), Err(ax25::Error::BadCall(_))));
}

fn l4_frames(bytes: &[u8], count: usize) -> Vec<Vec<u8>> {
    let ctrl = Control { reply: 0, compressed: false, hint: queue_hint(BW, L4, 0) };
    kiss::split(bytes, count, kiss::payload_field(L4)).iter().map(|f| kiss::frame(f, L4, ctrl, 0)).collect()
}

#[test]
fn eighteen_fragments_wrap_the_count_nibble() {
    let data = (0x80..=0xFF).map(|b| vec![b; 740]).find(|d| kiss::crc_nibble(&kiss::generic_encode(d)) & 0x10 != 0);
    let data = data.unwrap();
    let buf = Buffer { bytes: kiss::generic_encode(&data), aprs_type: AprsType::Text };
    let (level, fields) = kiss::fragments(&buf).unwrap();
    assert_eq!((level.get(), fields.len()), (4, kiss::MAX_FRAGMENTS));
    assert_eq!(fields[0][1] & 0x0F, 0);
    assert_eq!(receive(L4, &l4_frames(&buf.bytes, kiss::MAX_FRAGMENTS)), escape(2, &data));
}

fn l1_frames(bytes: &[u8]) -> Vec<Vec<u8>> {
    let ctrl = Control { reply: 0, compressed: false, hint: queue_hint(BW, Record::ONE, 0) };
    let field = kiss::payload_field(Record::ONE);
    kiss::split(bytes, 2, field).iter().map(|f| kiss::frame(f, Record::ONE, ctrl, 0)).collect()
}

fn reassemble(r: &mut Reassembler, level: Record, frames: &[Vec<u8>]) -> Vec<Push> {
    frames.iter().map(|f| r.push(f, level)).collect()
}

#[test]
fn reassembly_has_no_eighteen_fragment_cap() {
    let data = vec![0x80; 900];
    let long = kiss::generic_encode(&data);
    let frames = l4_frames(&long, 2);
    assert_eq!(frames.len(), 22);
    assert_eq!(receive(L4, &frames), escape(2, &data));
}

#[test]
fn reassembly_holds_at_most_5000_bytes() {
    let fits = kiss::generic_encode(&[0x80; kiss::RX_BUFFER - 1]);
    assert_eq!(fits.len(), kiss::RX_BUFFER);
    let mut r = Reassembler::default();
    assert_eq!(reassemble(&mut r, L4, &l4_frames(&fits, 2)).last(), Some(&Push::Done(fits.clone())));
    let over = kiss::generic_encode(&[0x80; kiss::RX_BUFFER]);
    let frames = l4_frames(&over, 2);
    let got = reassemble(&mut r, L4, &frames);
    let cut = got.iter().position(|p| *p == Push::Dropped).unwrap();
    assert_eq!(cut, frames.len() - 1);
    assert!(got[..cut].iter().all(|p| *p == Push::More));
    let bigger = kiss::generic_encode(&[0x80; kiss::RX_BUFFER + 200]);
    let frames = l4_frames(&bigger, 2);
    let got = reassemble(&mut r, L4, &frames);
    let cut = got.iter().position(|p| *p == Push::Dropped).unwrap();
    assert!(cut < frames.len() - 1);
    assert!(got[..cut].iter().all(|p| *p == Push::More));
    assert!(got[cut..].iter().all(|p| *p == Push::Dropped));
    let ok = kiss::generic_encode(b"after the flood");
    assert_eq!(reassemble(&mut r, L4, &l4_frames(&ok, 1)).last(), Some(&Push::Done(ok)));
}

#[test]
fn reassembly_holds_at_most_255_fragments() {
    let field = kiss::payload_field(Record::ONE);
    let len = |n: usize| 1 + (field - 2) + (n - 1) * (field - 1);
    let fits = kiss::generic_encode(&vec![0x80; len(kiss::RX_MAX_FRAGMENTS) - 1]);
    let frames = l1_frames(&fits);
    assert_eq!(frames.len(), kiss::RX_MAX_FRAGMENTS);
    assert!(fits.len() < kiss::RX_BUFFER);
    let mut r = Reassembler::default();
    assert_eq!(reassemble(&mut r, Record::ONE, &frames).last(), Some(&Push::Done(fits)));
    let over = kiss::generic_encode(&vec![0x80; len(kiss::RX_MAX_FRAGMENTS + 1) - 1]);
    let frames = l1_frames(&over);
    assert_eq!(frames.len(), kiss::RX_MAX_FRAGMENTS + 1);
    let got = reassemble(&mut r, Record::ONE, &frames);
    assert!(got[..kiss::RX_MAX_FRAGMENTS].iter().all(|p| *p == Push::More));
    assert_eq!(got.last(), Some(&Push::Dropped));
}

#[test]
fn reassembly_ignores_the_announced_count() {
    let data = kiss::generic_encode(&[b'q'; 150]);
    let frames = l4_frames(&data, 4);
    assert_eq!(frames.len(), 4);
    let mut r = Reassembler::default();
    assert_eq!(r.push(&frames[0], L4), Push::More);
    assert_eq!(r.push(&frames[1], L4), Push::More);
    assert_eq!(r.push(&frames[3], L4), Push::Dropped);
    let wrong = l4_frames(&data, 5);
    let mut r = Reassembler::default();
    let got: Vec<Push> = wrong.iter().map(|f| r.push(f, L4)).collect();
    assert_eq!(got.last(), Some(&Push::Done(data.clone())));
    let mut r = Reassembler::default();
    let got: Vec<Push> = frames.iter().map(|f| r.push(f, L4)).collect();
    assert_eq!(got.last(), Some(&Push::Done(data)));
}
