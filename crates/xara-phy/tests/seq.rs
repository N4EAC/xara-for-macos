mod common;
use common::*;
use xara_phy::seq;

fn bitstr(bits: &[u8]) -> String {
    bits.iter().map(|&b| char::from(b'0' + b)).collect()
}

fn packbits(bits: &[u8]) -> Vec<u8> {
    bits.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |a, (i, &b)| a | (b << (7 - i)))).collect()
}

fn mod128_bytes(rows: &[impl AsRef<[u8]>]) -> Vec<u8> {
    rows.iter().flat_map(|r| r.as_ref().iter().map(|&x| x % 128)).collect()
}

#[test]
fn whitening() {
    let c = load("sequences");
    let w = &c["whitening"];
    let bits = seq::whitening(seq::HF_SEED);
    assert_eq!(bits.len() as u64, w["n"].as_u64().unwrap());
    assert_eq!(bits.iter().map(|&b| u64::from(b)).sum::<u64>(), w["ones"].as_u64().unwrap());
    assert_eq!(bitstr(&bits[..256]), w["first_256"].as_str().unwrap());
    assert_eq!(bitstr(&bits[10000..10064]), w["bits_10000_10063"].as_str().unwrap());
    assert_eq!(bitstr(&bits[149936..]), w["bits_149936_149999"].as_str().unwrap());
    assert_eq!(sha256_hex(&packbits(&bits)), w["sha256_packbits"].as_str().unwrap());
    let fm = seq::whitening(seq::FM_SEED);
    assert_eq!(bitstr(&fm[..256]), c["fm_whitening_randomize43_first_256"].as_str().unwrap());
}

#[test]
fn psk_phasors() {
    let c = load("sequences");
    let v = &c["psk_phasor_index"];
    let d = seq::psk_phasor_index();
    for (row, want) in d.iter().zip(v["rows"].as_array().unwrap()) {
        let got: Vec<u32> = row.iter().map(|&x| u32::from(x % 128)).collect();
        assert_eq!(got, words(want));
    }
    assert_eq!(sha256_hex(&mod128_bytes(&d)), v["sha256_int8"].as_str().unwrap());
}

#[test]
fn data_dat_and_ofdm_phasors() {
    let c = load("sequences");
    let dd = seq::data_dat();
    assert_eq!(sha256_hex(&dd), c["data_dat"]["sha256"].as_str().unwrap());
    let v = &c["ofdm_phasor_index"];
    let d = seq::ofdm_phasor_index();
    assert_eq!(sha256_hex(&mod128_bytes(&d)), v["sha256_int8"].as_str().unwrap());
    for (k, want) in (8..13).zip(v["bins_8_12_m_0_15"].as_array().unwrap()) {
        let got: Vec<u32> = d[k][..16].iter().map(|&x| u32::from(x % 128)).collect();
        assert_eq!(got, words(want), "bin {k}");
    }
    let got: Vec<u32> = d[8][400..=420].iter().map(|&x| u32::from(x % 128)).collect();
    assert_eq!(got, words(&v["bin_8_m_400_420"]));
}
