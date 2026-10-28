//! Java "modified UTF-8", as used by `DataInput.readUTF` and therefore NBT.
//!
//! Differences from standard UTF-8: U+0000 is encoded as `C0 80`, and
//! supplementary characters are encoded as two three-byte surrogates.

pub fn encode(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut units = [0u16; 2];
    for c in s.chars() {
        for &unit in c.encode_utf16(&mut units).iter() {
            match unit {
                0x0001..=0x007f => out.push(unit as u8),
                0x0000 | 0x0080..=0x07ff => {
                    out.push(0xc0 | (unit >> 6) as u8);
                    out.push(0x80 | (unit & 0x3f) as u8);
                }
                _ => {
                    out.push(0xe0 | (unit >> 12) as u8);
                    out.push(0x80 | ((unit >> 6) & 0x3f) as u8);
                    out.push(0x80 | (unit & 0x3f) as u8);
                }
            }
        }
    }
    out
}

pub fn decode(bytes: &[u8]) -> Option<String> {
    let mut units = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let cont = |j: usize| {
            bytes
                .get(j)
                .filter(|&&c| c & 0xc0 == 0x80)
                .map(|&c| (c & 0x3f) as u16)
        };
        if b & 0x80 == 0 {
            units.push(b as u16);
            i += 1;
        } else if b & 0xe0 == 0xc0 {
            units.push(((b & 0x1f) as u16) << 6 | cont(i + 1)?);
            i += 2;
        } else if b & 0xf0 == 0xe0 {
            units.push(((b & 0x0f) as u16) << 12 | cont(i + 1)? << 6 | cont(i + 2)?);
            i += 3;
        } else {
            return None;
        }
    }
    // Lone surrogates can't be represented in a Rust String; replace them
    // rather than failing, as Minecraft itself tolerates them.
    Some(
        char::decode_utf16(units)
            .map(|r| r.unwrap_or('\u{fffd}'))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_java_encoding() {
        assert_eq!(encode("A"), b"A");
        assert_eq!(encode("\0"), [0xc0, 0x80]);
        assert_eq!(encode("é"), [0xc3, 0xa9]);
        // U+1F600 as surrogate pair D83D DE00
        assert_eq!(encode("\u{1F600}"), [0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80]);
    }

    #[test]
    fn round_trips() {
        for s in ["", "plain", "\0nul", "ü€\u{1F600}☃"] {
            assert_eq!(decode(&encode(s)).as_deref(), Some(s));
        }
    }
}
