mod common;
use common::*;
use xara_phy::dat::{fm_dat, hf_dat, hf_tables};
use xara_phy::fm::level::FmBandwidth;
use xara_phy::hf::level::Bandwidth;

fn u16be<T: Copy + TryInto<u16>>(v: &[T]) -> Vec<u8> {
    v.iter().flat_map(|&x| x.try_into().ok().expect("u16").to_be_bytes()).collect()
}

#[test]
fn hf_records() {
    let c = load("dat_tables");
    for bw in Bandwidth::ALL {
        let t = hf_tables(bw);
        for (r, rec) in c[bw.hz().to_string()]["records"].as_object().unwrap() {
            let t = &t[r.parse::<usize>().unwrap() - 1];
            let w = format!("{} r{r}", bw.hz());
            assert_eq!(sha256_hex(&u16be(&t.chan)), rec["sha256_chan_u16be"].as_str().unwrap(), "{w} chan");
            assert_eq!(sha256_hex(&u16be(&t.turbo)), rec["sha256_turbo_u16be"].as_str().unwrap(), "{w} turbo");
            assert_eq!(sha256_hex(&u16be(&t.other)), rec["sha256_other_u16be"].as_str().unwrap(), "{w} other");
        }
    }
}

#[test]
fn hf_files() {
    let c = load("dat_tables");
    for (bw, sha) in [
        (Bandwidth::Hz500, "020cf75a54c772ade64cdb45b52053beef3223b5bdb4847da1354d03627ed956"),
        (Bandwidth::Hz2300, "2c823ce719944dccf90194129b7c95f91b9add7c33a46bb0c80b8401d0edcfe5"),
        (Bandwidth::Hz2750, "e3e516f70f7710dc7992196573ce0141f5604d978e2701dced26eb06de3aede6"),
    ] {
        let v = &c[bw.hz().to_string()];
        let d = hf_dat(bw);
        assert_eq!(d.len() as u64, v["table_segment_len"].as_u64().unwrap());
        assert_eq!(u32::from(u16::from_be_bytes([d[d.len() - 2], d[d.len() - 1]])), hex_u32(&v["crc_stored_be"]));
        assert_eq!(sha256_hex(&d), sha, "{}", bw.hz());
    }
}

#[test]
fn fm_files() {
    for (bw, len, sha) in [
        (FmBandwidth::Narrow, 1_984_786, "ab420598450972b6050030d8762431b39c092a5a0bf17a3c03e413e578f1efa2"),
        (FmBandwidth::Wide, 3_302_010, "c777f8992ccf3fc8362d2311ff35e610cd478f36a1f31c1025b2466b3d2b7e3d"),
    ] {
        let d = fm_dat(bw);
        assert_eq!(d.len(), len, "{bw:?}");
        assert_eq!(sha256_hex(&d), sha, "{bw:?}");
    }
}
