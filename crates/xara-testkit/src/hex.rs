const DIGITS: &[u8; 16] = b"0123456789abcdef";

pub fn encode(data: impl AsRef<[u8]>) -> String {
    data.as_ref()
        .iter()
        .flat_map(|&b| [DIGITS[usize::from(b >> 4)] as char, DIGITS[usize::from(b & 15)] as char])
        .collect()
}

pub fn decode(text: impl AsRef<[u8]>) -> Result<Vec<u8>, String> {
    let t = text.as_ref();
    if t.len() % 2 != 0 {
        return Err(format!("odd hex length {}", t.len()));
    }
    let nibble = |i: usize| match t[i] {
        c @ b'0'..=b'9' => Ok(c - b'0'),
        c @ b'a'..=b'f' => Ok(c - b'a' + 10),
        c @ b'A'..=b'F' => Ok(c - b'A' + 10),
        c => Err(format!("bad hex digit {:?} at {i}", c as char)),
    };
    (0..t.len()).step_by(2).map(|i| Ok(nibble(i)? << 4 | nibble(i + 1)?)).collect()
}
