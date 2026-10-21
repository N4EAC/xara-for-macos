use xara_phy::call::unpack_call;
use xara_phy::crc::crc16_genibus;
use xara_phy::fm::frame::{parse_data, payload_len};
use xara_phy::fm::level::{DataLevel, Record};
use xara_phy::hf::ctrl::{Codebook, Fmt, NUM_CODES, Role};
use xara_phy::hf::frame::payload_capacity;
use xara_phy::hf::level::Bandwidth;
use xara_phy::hf::tx::Modulator;

fn sealed(body: &[u8]) -> Vec<u8> {
    let mut f = body.to_vec();
    f.extend(crc16_genibus(body).to_be_bytes());
    f
}

#[test]
fn short_frames_need_room_for_a_terminator() {
    for len in 0..8 {
        let f = sealed(&vec![0x14; len]);
        for record in [DataLevel::ONE.into(), Record::Start, Record::Broadcast] {
            for vias in [false, true] {
                let got = parse_data(record, &f, vias);
                match payload_len(record, f.len(), vias) {
                    Some(p) if p > 0 => assert_eq!(got.map(|d| d.data.len()), Some(p - 1), "{len} {record} {vias}"),
                    _ => assert_eq!(got, None, "{len} {record} {vias}"),
                }
                assert_eq!(parse_data(record, &f[..f.len().min(2)], vias), None);
            }
        }
    }
    assert_eq!(parse_data(Record::Start, &sealed(&[0x14, 0, 0]), true), None);
    assert_eq!(payload_len(Record::Start, 2, true), None);
    assert_eq!(payload_capacity(2, true), 0);
}

#[test]
fn short_call_field_does_not_unpack() {
    assert_eq!(unpack_call(&[0x39, 0xb0]), None);
    assert_eq!(unpack_call(&[0x39, 0xb0, 0xc1, 0x30, 0xc0, 0x00]).as_deref(), Some("N0CALL"));
}

#[test]
fn empty_transmission_is_silent_or_lead_in_only() {
    for bw in Bandwidth::ALL {
        let m = Modulator::new(bw);
        for record in bw.levels() {
            let l = bw.level(record);
            let x = m.transmission(record, &[], &mut |_| 0);
            if l.is_fsk() {
                assert!(x.is_empty(), "{bw:?} {record}");
                continue;
            }
            let frame = vec![0; l.frame_bytes() as usize];
            let one = m.transmission(record, &[&frame], &mut |_| 0);
            assert!(!x.is_empty() && x.len() * 10 < one.len(), "{bw:?} {record}: {} vs {}", x.len(), one.len());
            assert!(x.iter().all(|v| v.is_finite()));
        }
    }
}

#[test]
fn out_of_range_constellations_are_empty() {
    assert!(xara_phy::constellation::constellation(99).is_empty());
    assert!(xara_phy::fm::constellation::constellation(99).is_empty());
}

#[test]
fn extreme_codebook_seeds() {
    let shape = |cb: &Codebook| -> Vec<Vec<usize>> {
        Fmt::ALL
            .into_iter()
            .flat_map(|f| (1..=NUM_CODES as u32).flat_map(move |c| [cb.rx(f, c), cb.tx(f, c)]))
            .map(|w| w.iter().map(|s| s.len()).collect())
            .collect()
    };
    let max = || {
        let mut cb = Codebook::new(u32::MAX, [u32::MAX; 5], Role::Caller, Bandwidth::Hz2300);
        cb.set_connect_codes(u32::MAX, Role::Answerer);
        cb
    };
    let mut zero = Codebook::new(0, [0; 5], Role::Caller, Bandwidth::Hz2300);
    zero.set_connect_codes(0, Role::Answerer);
    assert_eq!(max(), max());
    assert_ne!(max(), zero);
    assert_eq!(shape(&max()), shape(&zero));

    let fm = xara_phy::fm::ctrl::Codebook::new(u32::MAX, u32::MAX);
    let fm0 = xara_phy::fm::ctrl::Codebook::new(0, 0);
    assert_eq!(fm, xara_phy::fm::ctrl::Codebook::new(u32::MAX, u32::MAX));
    assert!((1..=24).all(|c| fm.code(c) != fm0.code(c)));
}
