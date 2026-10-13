//! Base64 for Premiere's binary values (titles, colour mattes, masks) and a hand-built title
//! value used by tests; titles themselves are read and written by `prproj_title`.

/// Base64 (standard alphabet; whitespace skipped, `=` ends the data) → bytes. None on any other
/// character.
pub(crate) fn base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Bytes → base64 (standard alphabet, `=` padding).
pub(crate) fn base64_encode(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for chunk in b.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(A[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A `Source Text` value laid out like Premiere's for "50" (white fill, black 26 px stroke,
    /// FatFrank-Heavy at 399.41 px, centred in a 2125 × 539 box), built by hand.
    pub(crate) fn fifty() -> Vec<u8> {
        // FlatBuffer built back to front is tedious by hand; this one is assembled forwards with
        // absolute offsets and checked by the decoder test below.
        let mut b: Vec<u8> = Vec::new();
        let put32 = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        b.resize(400, 0);
        // root offset → root table at 16 (vtable at 8: size 6, table 8, field 0 at +4)
        put32(&mut b, 0, 16);
        b[8..14].copy_from_slice(&[6, 0, 8, 0, 4, 0]);
        put32(&mut b, 16, 8); // soffset to vtable (16 - 8)
        put32(&mut b, 20, 60 - 20); // field 0 → doc table at 60
        // doc vtable at 24: slots 0..5 (size 4 + 2*6 = 16, table 28)
        let vt = [16u16, 28, 4, 8, 12, 16, 20, 24];
        for (i, v) in vt.iter().enumerate() {
            b[24 + 2 * i..26 + 2 * i].copy_from_slice(&v.to_le_bytes());
        }
        put32(&mut b, 60, 60 - 24); // doc → its vtable
        put32(&mut b, 64, 120 - 64); // runs vector at 120
        put32(&mut b, 68, 200 - 68); // fonts vector at 200
        b[72..76].copy_from_slice(&2125f32.to_le_bytes());
        b[76..80].copy_from_slice(&539f32.to_le_bytes());
        put32(&mut b, 80, 2); // area text
        put32(&mut b, 84, 1); // centred
        // runs vector: 1 element → run table at 140
        put32(&mut b, 120, 1);
        put32(&mut b, 124, 140 - 124);
        // run vtable at 128: size 8, table 12, slots 0 (+4), 1 (+8)
        b[128..136].copy_from_slice(&[8, 0, 12, 0, 4, 0, 8, 0]);
        put32(&mut b, 140, 12); // → vtable at 128
        put32(&mut b, 144, 240 - 144); // text string at 240
        put32(&mut b, 148, 260 - 148); // style table at 260
        // fonts vector at 200: 1 string at 216
        put32(&mut b, 200, 1);
        put32(&mut b, 204, 216 - 204);
        put32(&mut b, 216, 14);
        b[220..234].copy_from_slice(b"FatFrank-Heavy");
        put32(&mut b, 240, 2);
        b[244..246].copy_from_slice(b"50");
        // style vtable at 300: slots 0..6 (size 18): 1 size +4, 4 stroke colour +8, 5 on +12, 6 width +16
        let svt = [18u16, 20, 0, 4, 0, 0, 8, 12, 16];
        for (i, v) in svt.iter().enumerate() {
            b[300 + 2 * i..302 + 2 * i].copy_from_slice(&v.to_le_bytes());
        }
        put32(&mut b, 260, (260i32 - 300) as u32); // negative soffset: vtable after the table
        b[264..268].copy_from_slice(&399.41f32.to_le_bytes());
        put32(&mut b, 268, 340 - 268); // stroke colour table at 340
        b[272] = 1;
        b[276..280].copy_from_slice(&26f32.to_le_bytes());
        // colour table at 340, vtable at 330 (size 10, table 8, slots r +4, g +5, b +6): black
        b[330..340].copy_from_slice(&[10, 0, 8, 0, 4, 0, 5, 0, 6, 0]);
        put32(&mut b, 340, 10);
        // the 12-byte header
        let mut out = (b.len() as u64).to_le_bytes().to_vec();
        out.extend_from_slice(&[0x44, 0x33, 0x22, 0x11]);
        out.extend_from_slice(&b);
        out
    }

    #[test]
    fn damaged_values_never_panic() {
        let good = fifty();
        for cut in 0..good.len() {
            let _ = crate::prproj_title::Tree::read(&good[..cut]);
        }
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..2000 {
            let mut b = good.clone();
            for _ in 0..4 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let i = 12 + (seed as usize) % (b.len() - 12);
                b[i] = (seed >> 32) as u8;
            }
            let _ = crate::prproj_title::Tree::read(&b);
        }
    }

    #[test]
    fn base64_round_trips() {
        for b in [&b""[..], b"O77\0\x01\0\0\0", b"\x01\0", b"abcd"] {
            assert_eq!(base64(&base64_encode(b)).unwrap(), b);
        }
        assert_eq!(base64_encode(&[0x4f, 0x37, 0x37, 0, 1, 0, 0, 0]), "Tzc3AAEAAAA=", "as Premiere writes a colour matte's");
    }

    #[test]
    fn base64_decodes() {
        assert_eq!(base64("AQA=").as_deref(), Some(&[1u8, 0][..]));
        assert_eq!(base64("SGVs\nbG8=").as_deref(), Some(&b"Hello"[..]));
        assert!(base64("not*base64").is_none());
    }
}
