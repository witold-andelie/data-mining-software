//! Strict RFC 4648 base64, with a decoded-size bound before allocation.
const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let a = chunk.first().copied().unwrap_or(0);
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        for index in [a >> 2, ((a & 3) << 4) | (b >> 4)] {
            if let Some(byte) = ALPHABET.get(usize::from(index)) {
                out.push(char::from(*byte));
            }
        }
        if chunk.len() > 1 {
            if let Some(byte) = ALPHABET.get(usize::from(((b & 15) << 2) | (c >> 6))) {
                out.push(char::from(*byte));
            }
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            if let Some(byte) = ALPHABET.get(usize::from(c & 63)) {
                out.push(char::from(*byte));
            }
        } else {
            out.push('=');
        }
    }
    out
}
pub(crate) fn decode(text: &str, limit: usize) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(4) {
        return Err("base64 length is not a multiple of four".into());
    }
    let padding = text.bytes().rev().take_while(|&b| b == b'=').count();
    if padding > 2 {
        return Err("invalid base64 padding".into());
    }
    let size = (text.len() / 4)
        .checked_mul(3)
        .and_then(|n| n.checked_sub(padding))
        .ok_or("base64 size overflow")?;
    if size > limit {
        return Err("decoded attachment exceeds limit".into());
    }
    let mut out = Vec::with_capacity(size);
    for (i, chunk) in text.as_bytes().chunks_exact(4).enumerate() {
        let mut value = 0u32;
        for (j, &byte) in chunk.iter().enumerate() {
            let digit = if byte == b'=' && i + 1 == text.len() / 4 && j >= 4 - padding {
                0
            } else {
                ALPHABET
                    .iter()
                    .position(|&b| b == byte)
                    .ok_or("invalid base64 character")? as u32
            };
            value = (value << 6) | digit;
        }
        if i + 1 == text.len() / 4
            && ((padding == 1 && value & 255 != 0) || (padding == 2 && value & 65535 != 0))
        {
            return Err("noncanonical base64 padding bits".into());
        }
        for shift in [16, 8, 0] {
            if out.len() < size {
                out.push((value >> shift) as u8);
            }
        }
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vectors_and_bounds() {
        for (raw, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(raw.as_bytes()), encoded);
            assert_eq!(decode(encoded, 100).unwrap(), raw.as_bytes());
        }
        let bytes: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&encode(&bytes), 256).unwrap(), bytes);
        for bad in [
            "=AAA", "A===", "AA=A", "Zg=A", "Zh==", "Zm9=", "AA", "@@@@", "====",
        ] {
            assert!(decode(bad, 100).is_err(), "{bad}");
        }
        assert!(decode("Zm9v", 2).is_err());
    }
}
