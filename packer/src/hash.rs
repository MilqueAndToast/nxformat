#[cfg(not(feature = "accelerated-sha"))]
pub(crate) type Sha256 = PortableSha256;

#[cfg(feature = "accelerated-sha")]
#[derive(Clone)]
pub(crate) struct Sha256(sha2::Sha256);
#[cfg(feature = "accelerated-sha")]
impl Sha256 {
    pub(crate) fn new() -> Self {
        use sha2::Digest;
        Self(sha2::Sha256::new())
    }
    pub(crate) fn update(&mut self, bytes: &[u8]) {
        use sha2::Digest;
        self.0.update(bytes);
    }
    pub(crate) fn finish(self) -> [u8; 32] {
        use sha2::Digest;
        self.0.finalize().into()
    }
}

/// Small dependency-free SHA-256 and CRC-32C implementation used by the
/// portable writer. SHA state is updated incrementally so file data is never
/// buffered as a complete logical file.
#[derive(Clone)]
#[cfg(any(test, not(feature = "accelerated-sha")))]
pub(crate) struct PortableSha256 {
    state: [u32; 8],
    block: [u8; 64],
    used: usize,
    total: u64,
}

#[cfg(any(test, not(feature = "accelerated-sha")))]
impl PortableSha256 {
    pub(crate) fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            block: [0; 64],
            used: 0,
            total: 0,
        }
    }

    pub(crate) fn update(&mut self, mut bytes: &[u8]) {
        self.total = self.total.wrapping_add(bytes.len() as u64);
        if self.used != 0 {
            let n = (64 - self.used).min(bytes.len());
            self.block[self.used..self.used + n].copy_from_slice(&bytes[..n]);
            self.used += n;
            bytes = &bytes[n..];
            if self.used == 64 {
                let block = self.block;
                self.compress(&block);
                self.used = 0;
            }
        }
        while bytes.len() >= 64 {
            let block: &[u8; 64] = bytes[..64].try_into().expect("fixed block size");
            self.compress(block);
            bytes = &bytes[64..];
        }
        if !bytes.is_empty() {
            self.block[..bytes.len()].copy_from_slice(bytes);
            self.used = bytes.len();
        }
    }

    pub(crate) fn finish(mut self) -> [u8; 32] {
        let bit_len = self.total.wrapping_mul(8);
        self.block[self.used] = 0x80;
        self.used += 1;
        if self.used > 56 {
            self.block[self.used..].fill(0);
            let block = self.block;
            self.compress(&block);
            self.block = [0; 64];
        } else {
            self.block[self.used..56].fill(0);
        }
        self.block[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let block = self.block;
        self.compress(&block);
        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().expect("word"));
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (dst, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *dst = dst.wrapping_add(value);
        }
    }
}

pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finish()
}

/// CRC-32C (Castagnoli). Uses the SSE4.2 or ARMv8 CRC instructions when the
/// CPU has them (all x86-64 since 2008, Apple Silicon, ARMv8.1+ Android) and
/// a slice-by-8 table otherwise.
pub(crate) fn crc32c(bytes: &[u8]) -> u32 {
    !crc32c_update(!0, bytes)
}

/// Raw CRC-32C register update (no pre/post inversion).
pub(crate) fn crc32c_update(crc: u32, bytes: &[u8]) -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("sse4.2") {
            // SAFETY: the feature was detected at run time.
            return unsafe { crc32c_x86(crc, bytes) };
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        if std::arch::is_aarch64_feature_detected!("crc") {
            // SAFETY: the feature was detected at run time.
            return unsafe { crc32c_arm(crc, bytes) };
        }
    }
    crc32c_table(crc, bytes)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.2")]
unsafe fn crc32c_x86(mut crc: u32, bytes: &[u8]) -> u32 {
    use std::arch::x86_64::{_mm_crc32_u64, _mm_crc32_u8};
    let mut words = bytes.chunks_exact(8);
    let mut wide = crc as u64;
    for word in &mut words {
        wide = _mm_crc32_u64(wide, u64::from_le_bytes(word.try_into().unwrap()));
    }
    crc = wide as u32;
    for &b in words.remainder() {
        crc = _mm_crc32_u8(crc, b);
    }
    crc
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "crc")]
unsafe fn crc32c_arm(mut crc: u32, bytes: &[u8]) -> u32 {
    use std::arch::aarch64::{__crc32cb, __crc32cd};
    let mut words = bytes.chunks_exact(8);
    for word in &mut words {
        crc = __crc32cd(crc, u64::from_le_bytes(word.try_into().unwrap()));
    }
    for &b in words.remainder() {
        crc = __crc32cb(crc, b);
    }
    crc
}

const fn crc32c_tables() -> [[u32; 256]; 8] {
    let mut t = [[0u32; 256]; 8];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = (c >> 1) ^ (0x82f6_3b78 & (0u32.wrapping_sub(c & 1)));
            k += 1;
        }
        t[0][i] = c;
        i += 1;
    }
    let mut n = 1;
    while n < 8 {
        let mut i = 0;
        while i < 256 {
            let prev = t[n - 1][i];
            t[n][i] = (prev >> 8) ^ t[0][(prev & 0xff) as usize];
            i += 1;
        }
        n += 1;
    }
    t
}
static CRC32C_TABLES: [[u32; 256]; 8] = crc32c_tables();

pub(crate) fn crc32c_table(mut crc: u32, bytes: &[u8]) -> u32 {
    let t = &CRC32C_TABLES;
    let mut words = bytes.chunks_exact(8);
    for w in &mut words {
        let lo = crc ^ u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
        crc = t[7][(lo & 0xff) as usize]
            ^ t[6][((lo >> 8) & 0xff) as usize]
            ^ t[5][((lo >> 16) & 0xff) as usize]
            ^ t[4][(lo >> 24) as usize]
            ^ t[3][w[4] as usize]
            ^ t[2][w[5] as usize]
            ^ t[1][w[6] as usize]
            ^ t[0][w[7] as usize];
    }
    for &b in words.remainder() {
        crc = (crc >> 8) ^ t[0][((crc ^ b as u32) & 0xff) as usize];
    }
    crc
}

/// Integrity slice size of NX 0.2: one CRC-32C per 16 KiB of logical bytes.
pub(crate) const CHECK_SIZE: usize = 16 * 1024;

pub(crate) fn check_count(logical: usize) -> usize {
    logical.div_ceil(CHECK_SIZE)
}

/// Append one CRC-32C per 16 KiB slice of `bytes` to `out`.
pub(crate) fn slice_checks(bytes: &[u8], out: &mut Vec<u32>) {
    // Independent slices can use independent CRC dependency chains. Interleave
    // eight of them so the CPU can overlap instructions instead of waiting for
    // the preceding word's CRC. Keep the wire checks in their original order.
    #[cfg(target_arch = "x86_64")]
    if bytes.len() >= 8 * CHECK_SIZE && std::arch::is_x86_feature_detected!("sse4.2") {
        // SAFETY: the feature was detected at run time.
        return unsafe { slice_checks_x86(bytes, out) };
    }
    #[cfg(target_arch = "aarch64")]
    if bytes.len() >= 8 * CHECK_SIZE && std::arch::is_aarch64_feature_detected!("crc") {
        // SAFETY: the feature was detected at run time.
        return unsafe { slice_checks_arm(bytes, out) };
    }
    out.extend(bytes.chunks(CHECK_SIZE).map(crc32c));
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.2")]
unsafe fn slice_checks_x86(bytes: &[u8], out: &mut Vec<u32>) {
    use std::arch::x86_64::_mm_crc32_u64;
    let mut groups = bytes.chunks_exact(8 * CHECK_SIZE);
    for group in &mut groups {
        let mut crc = [!0u32 as u64; 8];
        for at in (0..CHECK_SIZE).step_by(8) {
            for (lane, state) in crc.iter_mut().enumerate() {
                // SAFETY: a complete group has eight CHECK_SIZE-byte slices;
                // at + 8 <= CHECK_SIZE. Unaligned input is explicitly allowed.
                let word = unsafe {
                    group
                        .as_ptr()
                        .add(lane * CHECK_SIZE + at)
                        .cast::<u64>()
                        .read_unaligned()
                };
                *state = _mm_crc32_u64(*state, u64::from_le(word));
            }
        }
        out.extend(crc.map(|state| !(state as u32)));
    }
    out.extend(groups.remainder().chunks(CHECK_SIZE).map(crc32c));
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "crc")]
unsafe fn slice_checks_arm(bytes: &[u8], out: &mut Vec<u32>) {
    use std::arch::aarch64::__crc32cd;
    let mut groups = bytes.chunks_exact(8 * CHECK_SIZE);
    for group in &mut groups {
        let mut crc = [!0u32; 8];
        for at in (0..CHECK_SIZE).step_by(8) {
            for (lane, state) in crc.iter_mut().enumerate() {
                // SAFETY: a complete group has eight CHECK_SIZE-byte slices;
                // at + 8 <= CHECK_SIZE. Unaligned input is explicitly allowed.
                let word = unsafe {
                    group
                        .as_ptr()
                        .add(lane * CHECK_SIZE + at)
                        .cast::<u64>()
                        .read_unaligned()
                };
                *state = __crc32cd(*state, u64::from_le(word));
            }
        }
        out.extend(crc.map(|state| !state));
    }
    out.extend(groups.remainder().chunks(CHECK_SIZE).map(crc32c));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn from_hex(s: &str) -> Vec<u8> {
        s.as_bytes()
            .chunks_exact(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect()
    }
    #[test]
    fn sha256_independent_vectors() {
        assert_eq!(
            Sha256::new().finish().to_vec(),
            from_hex("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        assert_eq!(
            sha256(b"abc").to_vec(),
            from_hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        let mut h = Sha256::new();
        h.update(b"a");
        h.update(b"bc");
        assert_eq!(
            h.finish().to_vec(),
            from_hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
    }
    #[test]
    fn crc32c_standard_check() {
        assert_eq!(crc32c(b"123456789"), 0xe3069283);
        assert_eq!(!crc32c_table(!0, b"123456789"), 0xe3069283);
        let bytes: Vec<u8> = (0..70_001u32).map(|i| (i * 7 + i / 13) as u8).collect();
        for len in [0, 1, 7, 8, 9, 63, 4096, 70_001] {
            assert_eq!(
                crc32c(&bytes[..len]),
                !crc32c_table(!0, &bytes[..len]),
                "length {len}"
            );
        }
    }

    #[test]
    fn interleaved_slice_checks_match_portable_crc_with_tails_and_unaligned_input() {
        let bytes: Vec<u8> = (0..1024 * 1024 + 8u32)
            .map(|i| (i.wrapping_mul(17) ^ (i >> 11)) as u8)
            .collect();
        for start in 0..8 {
            for len in [
                0,
                1,
                CHECK_SIZE - 1,
                CHECK_SIZE,
                8 * CHECK_SIZE - 1,
                8 * CHECK_SIZE,
                8 * CHECK_SIZE + 1,
                9 * CHECK_SIZE,
                1024 * 1024,
            ] {
                let input = &bytes[start..start + len];
                let mut actual = vec![0x1234_5678];
                slice_checks(input, &mut actual);
                let expected: Vec<_> = std::iter::once(0x1234_5678)
                    .chain(
                        input
                            .chunks(CHECK_SIZE)
                            .map(|slice| !crc32c_table(!0, slice)),
                    )
                    .collect();
                assert_eq!(actual, expected, "offset {start}, length {len}");
            }
        }
    }
}

#[cfg(all(test, feature = "accelerated-sha"))]
#[test]
fn accelerated_and_portable_match_at_chunk_boundaries() {
    for size in [0, 1, 55, 56, 63, 64, 65, 262144, 1048576] {
        let bytes: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let mut portable = PortableSha256::new();
        let mut accelerated = Sha256::new();
        for chunk in bytes.chunks(37) {
            portable.update(chunk);
            accelerated.update(chunk);
        }
        assert_eq!(portable.finish(), accelerated.finish());
    }
}
