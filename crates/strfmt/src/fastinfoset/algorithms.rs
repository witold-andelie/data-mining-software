//! Built-in encoding algorithms (X.891 clause 10): octets back to the character string.
//! Lists of values ("words") are separated by a single SPACE.

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn decode(index: u32, octets: &[u8]) -> Result<String, &'static str> {
    match index {
        1 => Ok(octets.iter().map(|octet| format!("{octet:02X}")).collect()),
        2 => Ok(base64(octets)),
        3 => words::<2>(octets, |b| i16::from_be_bytes(b).to_string()),
        4 => words::<4>(octets, |b| i32::from_be_bytes(b).to_string()),
        5 => words::<8>(octets, |b| i64::from_be_bytes(b).to_string()),
        6 => boolean(octets),
        7 => words::<4>(octets, |b| {
            canonical_float(f64::from(f32::from_be_bytes(b)), true)
        }),
        8 => words::<8>(octets, |b| canonical_float(f64::from_be_bytes(b), false)),
        9 => words::<16>(octets, uuid),
        10 => std::str::from_utf8(octets)
            .map(str::to_string)
            .map_err(|_| "cdata is not UTF-8"),
        _ => Err("not a built-in encoding algorithm"),
    }
}

/// Fixed-size big-endian values (10.4 to 10.6, 10.8 to 10.10); at least one word is required.
fn words<const N: usize>(
    octets: &[u8],
    format: impl Fn([u8; N]) -> String,
) -> Result<String, &'static str> {
    if octets.is_empty() || !octets.len().is_multiple_of(N) {
        return Err("length is not a positive multiple of the value size");
    }
    let mut out = Vec::with_capacity(octets.len() / N);
    for chunk in octets.chunks_exact(N) {
        let value: [u8; N] = chunk.try_into().map_err(|_| "value size mismatch")?;
        out.push(format(value));
    }
    Ok(out.join(" "))
}

/// 10.7: the first four bits count unused bits in the last octet; values start on bit 5.
fn boolean(octets: &[u8]) -> Result<String, &'static str> {
    let first = *octets.first().ok_or("empty boolean data")?;
    let unused = usize::from(first >> 4);
    let capacity = octets.len() * 8 - 4;
    if unused > 7 || unused >= capacity {
        return Err("invalid unused-bit count");
    }
    let count = capacity - unused;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let index = i + 4;
        let octet = octets.get(index / 8).ok_or("boolean data truncated")?;
        out.push(if (octet >> (7 - index % 8)) & 1 == 1 {
            "true"
        } else {
            "false"
        });
    }
    Ok(out.join(" "))
}

/// W3C XML Schema Part 2 canonical float/double (3.2.4.2): `d.dddE±x`, INF, -INF, NaN.
fn canonical_float(value: f64, single: bool) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 {
            "INF".into()
        } else {
            "-INF".into()
        };
    }
    // Shortest round-trip digits for the original precision.
    let raw = if single {
        format!("{:e}", value as f32)
    } else {
        format!("{value:e}")
    };
    let (mantissa, exponent) = raw.split_once('e').unwrap_or((raw.as_str(), "0"));
    let mantissa = if mantissa.contains('.') {
        mantissa.to_string()
    } else {
        format!("{mantissa}.0")
    };
    format!("{mantissa}E{exponent}")
}

/// 10.10: X.667 hexadecimal form, lower case, 8-4-4-4-12.
fn uuid(bytes: [u8; 16]) -> String {
    let hex: String = bytes.iter().map(|octet| format!("{octet:02x}")).collect();
    let part = |range: std::ops::Range<usize>| hex.get(range).unwrap_or_default().to_string();
    format!(
        "{}-{}-{}-{}-{}",
        part(0..8),
        part(8..12),
        part(12..16),
        part(16..20),
        part(20..32)
    )
}

/// RFC 2045 Base64 without line breaks (X.891 10.3 allows the whitespace to be omitted).
fn base64(octets: &[u8]) -> String {
    let mut out = String::with_capacity(octets.len().div_ceil(3) * 4);
    for chunk in octets.chunks(3) {
        let b0 = chunk.first().copied().unwrap_or(0);
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let triple = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        for (position, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if position <= chunk.len() {
                let index = ((triple >> shift) & 0x3F) as usize;
                out.push(char::from(BASE64.get(index).copied().unwrap_or(b'=')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_algorithms_decode_lists() {
        assert_eq!(decode(3, &[0xFF, 0xFE, 0x00, 0x07]).unwrap(), "-2 7");
        assert_eq!(decode(4, &[0x80, 0, 0, 0]).unwrap(), "-2147483648");
        assert_eq!(decode(5, &[0, 0, 0, 0, 0, 0, 0, 42]).unwrap(), "42");
        assert!(decode(4, &[1, 2, 3]).is_err());
        assert!(decode(3, &[]).is_err());
    }

    #[test]
    fn boolean_uses_the_unused_bit_count() {
        // unused = 1, values on bits 5..7 of the only octet: 1 0 1
        assert_eq!(decode(6, &[0b0001_1010]).unwrap(), "true false true");
        // unused = 0 over two octets: 4 + 8 values
        assert_eq!(
            decode(6, &[0b0000_0001, 0b1000_0000]).unwrap(),
            "false false false true true false false false false false false false"
        );
        assert!(decode(6, &[0b0100_0000]).is_err());
        assert!(decode(6, &[]).is_err());
    }

    #[test]
    fn floats_use_schema_canonical_form() {
        assert_eq!(decode(7, &1.5f32.to_be_bytes()).unwrap(), "1.5E0");
        assert_eq!(decode(7, &100f32.to_be_bytes()).unwrap(), "1.0E2");
        assert_eq!(decode(7, &0.1f32.to_be_bytes()).unwrap(), "1.0E-1");
        assert_eq!(decode(8, &(-1.5e300f64).to_be_bytes()).unwrap(), "-1.5E300");
        assert_eq!(decode(8, &0f64.to_be_bytes()).unwrap(), "0.0E0");
        assert_eq!(decode(8, &f64::NAN.to_be_bytes()).unwrap(), "NaN");
        assert_eq!(decode(7, &f32::NEG_INFINITY.to_be_bytes()).unwrap(), "-INF");
    }

    #[test]
    fn hex_base64_uuid_and_cdata() {
        assert_eq!(decode(1, &[0x0A, 0xFF]).unwrap(), "0AFF");
        assert_eq!(decode(2, b"Ma").unwrap(), "TWE=");
        assert_eq!(decode(2, b"Man").unwrap(), "TWFu");
        assert_eq!(decode(2, b"M").unwrap(), "TQ==");
        let id: Vec<u8> = (0u8..16).collect();
        assert_eq!(
            decode(9, &id).unwrap(),
            "00010203-0405-0607-0809-0a0b0c0d0e0f"
        );
        assert_eq!(decode(10, "a<b".as_bytes()).unwrap(), "a<b");
        assert!(decode(11, &[1]).is_err());
    }
}
