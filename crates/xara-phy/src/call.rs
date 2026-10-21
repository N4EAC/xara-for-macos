use crate::crc::crc16_genibus;

pub fn call_hash(call: &[u8]) -> u32 {
    if call.is_empty() {
        return 0;
    }
    let c1 = u32::from(crc16_genibus(call));
    let c2 = u32::from(crc16_genibus(&call[call.len().saturating_sub(3)..]));
    ((c2 >> 8) << 16) | c1
}

pub fn pack_call(call: &[u8]) -> [u8; 6] {
    let mut bits: u64 = 0;
    for i in 0..8 {
        let code = match call.get(i).copied().unwrap_or(b' ') {
            c @ b'A'..=b'Z' => c - b'A' + 1,
            c @ b'0'..=b'9' => c - b'0' + 27,
            _ => 0,
        };
        bits = (bits << 6) | u64::from(code);
    }
    bits.to_be_bytes()[2..].try_into().unwrap()
}

pub fn is_us_call(call: &str) -> bool {
    let base = call.split('-').next().unwrap_or("").as_bytes();
    let n = base.len();
    let digits: Vec<usize> = (0..n).filter(|&i| base[i].is_ascii_digit()).collect();
    if !(4..=6).contains(&n) || !b"NKWA".contains(&base[0]) || digits.len() != 1 || !(1..=2).contains(&digits[0]) {
        return false;
    }
    if !base.iter().all(u8::is_ascii_alphanumeric) {
        return false;
    }
    if base[0] == b'A' && (b'M'..=b'Z').contains(&base[1]) {
        return false;
    }
    let (pre, suf) = (digits[0], n - digits[0] - 1);
    match (pre, suf) {
        (2, 3) => matches!(base[0], b'K' | b'W'),
        (2, 2) if base[0] == b'A' => (b'A'..=b'L').contains(&base[1]),
        _ => true,
    }
}

pub fn unpack_call(b: &[u8]) -> Option<String> {
    let b = b.first_chunk::<6>()?;
    let mut v = 0u64;
    for &x in b {
        v = (v << 8) | u64::from(x);
    }
    let s: String = (0..8)
        .rev()
        .map(|i| match ((v >> (6 * i)) & 63) as u8 {
            c @ 1..=26 => (b'A' + c - 1) as char,
            c @ 27..=36 => (b'0' + c - 27) as char,
            _ => ' ',
        })
        .collect();
    Some(s.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_examples() {
        assert_eq!(call_hash(b"N0CALL"), 0xD80986);
        assert_eq!(call_hash(b""), 0);
        assert_eq!(pack_call(b"N0CALL"), [0x39, 0xb0, 0xc1, 0x30, 0xc0, 0x00]);
    }
}
