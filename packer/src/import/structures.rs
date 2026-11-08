//! Explicit, platform-independent structural profiles; no RSA authentication.
use super::{invalid, unsupported, ImportLimits};
use crate::hash::Sha256;
use crate::{CancellationToken, WriterError};
use std::io::{Read, Seek, SeekFrom};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NpdmProfile {
    Absent,
    MetaAcid2048V1,
    MetaAcid3072V1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NpdmFacts {
    pub program_id: u64,
    pub flags: u8,
    pub remote_memory_size: u32,
    pub profile: NpdmProfile,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NsoFacts {
    pub build_id: Option<[u8; 32]>,
    pub segment_hashes_checked: u8,
    pub decoded_bytes: u64,
    /// ZBIC segments: ranges and magic checked, not decoded, SHA not checked.
    pub zbic_segments: u8,
}
/// NSO flag bit 7: compressed segments use ZBIC (Zstandard with its own
/// entropy-table coding) instead of LZ4.
const NSO_ZBIC: u32 = 0x80;

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
fn region(at: u64, size: u64, limit: u64) -> Result<(u64, u64), WriterError> {
    if at > limit || size > limit - at {
        return Err(invalid("native structure range exceeds its parent"));
    }
    Ok((at, at + size))
}
fn disjoint(ranges: &mut [(u64, u64)]) -> Result<(), WriterError> {
    ranges.sort_unstable();
    if ranges.windows(2).any(|w| w[0].1 > w[1].0) {
        return Err(invalid("native structure regions overlap"));
    }
    Ok(())
}

/// Validate the explicitly selected META/ACI0/ACID envelope and bounded access
/// tables. Permissions and signatures are not authenticated or executed.
pub fn validate_npdm(bytes: &[u8], profile: NpdmProfile) -> Result<NpdmFacts, WriterError> {
    if bytes.len() < 0x80 || bytes.len() > 1024 * 1024 || &bytes[..4] != b"META" {
        return Err(invalid("NPDM lacks a bounded META header"));
    }
    let magic = match profile {
        NpdmProfile::MetaAcid2048V1 => 0x200,
        NpdmProfile::MetaAcid3072V1 => 0x300,
        NpdmProfile::Absent => return Err(unsupported("NPDM present with absent profile")),
    };
    if bytes[8..12] != [0; 4]
        || bytes[0x40..0x70].iter().any(|b| *b != 0)
        || bytes[0x0d] != 0
        || bytes[0x0c] & 0x80 != 0
        || (bytes[0x0c] >> 1) & 7 > 3
    {
        return Err(unsupported(
            "unsupported NPDM flags, address space or reserved fields",
        ));
    }
    if bytes[0x0e] > 63 || u32_at(bytes, 0x1c) % 0x1000 != 0 {
        return Err(invalid("NPDM priority or stack alignment invalid"));
    }
    // RemoteMemorySize is a documented raw field, never a platform detector or
    // an instruction to allocate guest memory in this importer.
    let aci_at = u32_at(bytes, 0x70) as u64;
    let aci_len = u32_at(bytes, 0x74) as u64;
    let acid_at = u32_at(bytes, 0x78) as u64;
    let acid_len = u32_at(bytes, 0x7c) as u64;
    if aci_at < 0x80
        || acid_at < 0x80
        || aci_at % 4 != 0
        || acid_at % 4 != 0
        || aci_len < 0x40
        || acid_len < (magic + 0x40) as u64
    {
        return Err(invalid("NPDM ACI0/ACID header bounds invalid"));
    }
    let mut ranges = [
        region(aci_at, aci_len, bytes.len() as u64)?,
        region(acid_at, acid_len, bytes.len() as u64)?,
    ];
    disjoint(&mut ranges)?;
    let aci = &bytes[aci_at as usize..(aci_at + aci_len) as usize];
    let acid = &bytes[acid_at as usize..(acid_at + acid_len) as usize];
    if &aci[..4] != b"ACI0" || &acid[magic..magic + 4] != b"ACID" {
        return Err(unsupported(
            "NPDM does not match the declared ACI0/ACID profile",
        ));
    }
    if u32_at(acid, magic + 4) as u64 != acid_len - (magic / 2) as u64 {
        return Err(invalid(
            "ACID signed region size disagrees with its envelope",
        ));
    }
    if aci[4..0x10]
        .iter()
        .chain(&aci[0x18..0x20])
        .chain(&aci[0x38..0x40])
        .any(|b| *b != 0)
        || acid[magic + 0xa..magic + 0xc] != [0; 2]
        || acid[magic + 0x38..magic + 0x40].iter().any(|b| *b != 0)
    {
        return Err(unsupported("unsupported ACI0/ACID reserved fields"));
    }
    if u32_at(acid, magic + 0xc) & !0xff != 0 {
        return Err(unsupported("unsupported ACID flags"));
    }
    let id = u64_at(aci, 0x10);
    if id == 0 || id < u64_at(acid, magic + 0x10) || id > u64_at(acid, magic + 0x18) {
        return Err(invalid("ACI0 Program ID is outside ACID range"));
    }
    for (b, table, floor) in [(aci, 0x20, 0x40), (acid, magic + 0x20, magic + 0x40)] {
        let mut children = Vec::new();
        for i in 0..3 {
            let at = u32_at(b, table + i * 8) as u64;
            let size = u32_at(b, table + i * 8 + 4) as u64;
            if size == 0 {
                if i == 0 {
                    return Err(invalid("NPDM requires a complete file-access header"));
                }
                if at != 0 {
                    region(at, 0, b.len() as u64)?;
                }
                continue;
            }
            if at < floor as u64 || (i == 2 && (at % 4 != 0 || size % 4 != 0)) {
                return Err(invalid("NPDM access table alignment/header overlap"));
            }
            if i == 0 && size < if floor == 0x40 { 0x1c } else { 0x2c } {
                return Err(invalid("NPDM file access table is truncated"));
            }
            children.push(region(at, size, b.len() as u64)?);
            if i == 0 && b[at as usize] != 1 {
                return Err(unsupported("NPDM file-access header version"));
            }
        }
        disjoint(&mut children)?;
    }
    Ok(NpdmFacts {
        program_id: id,
        flags: bytes[0xc],
        remote_memory_size: u32_at(bytes, 0x10),
        profile,
    })
}

struct Lz4Output {
    ring: Box<[u8; 65536]>,
    size: u64,
    limit: u64,
    hash: Sha256,
}
impl Lz4Output {
    fn emit(&mut self, byte: u8) -> Result<(), WriterError> {
        if self.size == self.limit {
            return Err(invalid("LZ4 expansion exceeds NSO segment"));
        }
        self.ring[self.size as usize & 65535] = byte;
        self.size += 1;
        if self.size & 65535 == 0 {
            self.hash.update(self.ring.as_ref());
        }
        Ok(())
    }
    fn finish(mut self) -> Result<[u8; 32], WriterError> {
        if self.size != self.limit {
            return Err(invalid("NSO decoded segment length mismatch"));
        }
        self.hash.update(&self.ring[..self.size as usize & 65535]);
        Ok(self.hash.finish())
    }
}
fn byte(r: &mut impl Read) -> Result<u8, WriterError> {
    let mut b = [0];
    r.read_exact(&mut b)?;
    Ok(b[0])
}
fn length(r: &mut impl Read, initial: u8, limit: u64) -> Result<u64, WriterError> {
    let mut n = initial as u64;
    if initial == 15 {
        loop {
            let b = byte(r)?;
            n = n
                .checked_add(b as u64)
                .ok_or_else(|| invalid("LZ4 length overflow"))?;
            if n > limit {
                return Err(invalid("LZ4 length exceeds segment budget"));
            }
            if b != 255 {
                break;
            }
        }
    }
    Ok(n)
}
fn lz4(r: &mut impl Read, size: u64, token: &CancellationToken) -> Result<[u8; 32], WriterError> {
    let mut out = Lz4Output {
        ring: Box::new([0; 65536]),
        size: 0,
        limit: size,
        hash: Sha256::new(),
    };
    // A buffered bounded source prevents one-byte system calls. The 64 KiB
    // history handles overlapping matches without a segment-sized allocation.
    let mut last_match_start = None;
    loop {
        super::cancelled(token)?;
        let t = byte(r)?;
        let literals = length(r, t >> 4, size)?;
        for i in 0..literals {
            if i & 65535 == 0 {
                super::cancelled(token)?;
            }
            out.emit(byte(r)?)?;
        }
        let mut first = [0];
        if r.read(&mut first)? == 0 {
            // The last token's low nibble is unused by the LZ4 block spec.
            if literals < size.min(5) || last_match_start.is_some_and(|at| size - at < 12) {
                return Err(invalid("invalid final LZ4 sequence"));
            }
            return out.finish();
        }
        let offset = first[0] as u64 | ((byte(r)? as u64) << 8);
        if offset == 0 || offset > out.size {
            return Err(invalid("invalid LZ4 back reference"));
        }
        last_match_start = Some(out.size);
        let count = length(r, t & 15, size)?
            .checked_add(4)
            .ok_or_else(|| invalid("LZ4 match overflow"))?;
        for i in 0..count {
            if i & 65535 == 0 {
                super::cancelled(token)?;
            }
            let b = out.ring[(out.size - offset) as usize & 65535];
            out.emit(b)?;
        }
    }
}

/// NSO0 version zero, RAW/LZ4/ZBIC. Defined segment SHA flags are enforced
/// for RAW and LZ4. ZBIC segments are kept as stored: their ranges and magic
/// are checked, their content is not decoded.
/// Build ID is an observed field, not proof of successful execution/authorship.
pub fn validate_nso<R: Read + Seek>(
    r: &mut R,
    size: u64,
    limits: &ImportLimits,
    token: &CancellationToken,
) -> Result<NsoFacts, WriterError> {
    limits.validate()?;
    super::cancelled(token)?;
    if size > limits.max_file_bytes {
        return Err(super::limit("NSO file exceeds input budget"));
    }
    if size < 0x100 {
        return Err(invalid("NSO header is truncated"));
    }
    let mut h = [0; 0x100];
    r.seek(SeekFrom::Start(0))?;
    r.read_exact(&mut h)?;
    if &h[..4] != b"NSO0"
        || u32_at(&h, 4) != 0
        || u32_at(&h, 8) != 0
        || h[0x6c..0x88].iter().any(|b| *b != 0)
        || u32_at(&h, 12) & !0xff != 0
    {
        return Err(unsupported("unsupported NSO revision/flags"));
    }
    let flags = u32_at(&h, 12);
    if flags & NSO_ZBIC != 0 && flags & 7 == 0 {
        return Err(invalid("ZBIC NSO has no compressed segment"));
    }
    let mut disk = Vec::new();
    let mut memory = Vec::new();
    let mut decoded = 0u64;
    let name_size = u32_at(&h, 0x2c) as u64;
    if name_size != 0 {
        let at = u32_at(&h, 0x1c) as u64;
        if at < 0x100 {
            return Err(invalid("NSO module name overlaps header"));
        }
        disk.push(region(at, name_size, size)?);
    }
    let mut segments = Vec::new();
    for i in 0..3 {
        let at = u32_at(&h, 0x10 + i * 16) as u64;
        let mem = u32_at(&h, 0x14 + i * 16) as u64;
        let logical = u32_at(&h, 0x18 + i * 16) as u64;
        let stored = u32_at(&h, 0x60 + i * 4) as u64;
        if logical > limits.max_segment_bytes
            || stored > limits.max_segment_bytes
            || mem % 0x1000 != 0
        {
            return Err(unsupported(
                "NSO segment exceeds supported limits/alignment",
            ));
        }
        if logical == 0 {
            if stored != 0 {
                return Err(invalid("empty NSO segment has payload"));
            }
            continue;
        }
        if at < 0x100 || stored == 0 || (flags & (1 << i) == 0 && stored != logical) {
            return Err(invalid("invalid NSO segment size/offset"));
        }
        disk.push(region(at, stored, size)?);
        let bss = if i == 2 { u32_at(&h, 0x3c) as u64 } else { 0 };
        memory.push(region(mem, logical + bss, limits.max_image_bytes)?);
        decoded = decoded
            .checked_add(logical)
            .ok_or_else(|| invalid("NSO image overflow"))?;
        segments.push((i, at, stored, logical));
    }
    if segments.first().map(|s| s.0) != Some(0) {
        return Err(invalid("NSO lacks text segment"));
    }
    if decoded > limits.max_executable_bytes {
        return Err(super::limit("NSO decoded work exceeds budget"));
    }
    disjoint(&mut disk)?;
    disjoint(&mut memory)?;
    let ro = u32_at(&h, 0x28) as u64;
    for at in [0x88, 0x90, 0x98] {
        region(u32_at(&h, at) as u64, u32_at(&h, at + 4) as u64, ro)?;
    }
    let mut checked = 0;
    let mut zbic = 0;
    let mut buffer = [0; 65536];
    for (i, at, stored, logical) in segments {
        super::cancelled(token)?;
        r.seek(SeekFrom::Start(at))?;
        if flags & NSO_ZBIC != 0 && flags & (1 << i) != 0 {
            let mut magic = [0; 4];
            if stored < 4 || r.read_exact(&mut magic).is_err() || &magic != b"ZBIC" {
                return Err(invalid("ZBIC NSO segment lacks the ZBIC magic"));
            }
            zbic += 1;
            continue;
        }
        let mut bounded = std::io::BufReader::with_capacity(65536, r.take(stored));
        let actual = if flags & (1 << i) != 0 {
            lz4(&mut bounded, logical, token)?
        } else {
            let mut hash = Sha256::new();
            let mut left = logical;
            while left != 0 {
                super::cancelled(token)?;
                let n = left.min(buffer.len() as u64) as usize;
                bounded.read_exact(&mut buffer[..n])?;
                hash.update(&buffer[..n]);
                left -= n as u64;
            }
            hash.finish()
        };
        if flags & (1 << (i + 3)) != 0 {
            if actual.as_slice() != &h[0xa0 + i * 32..0xc0 + i * 32] {
                return Err(invalid("NSO segment SHA-256 mismatch"));
            }
            checked += 1;
        }
    }
    let mut build = [0; 32];
    build.copy_from_slice(&h[0x40..0x60]);
    Ok(NsoFacts {
        build_id: if build == [0; 32] { None } else { Some(build) },
        segment_hashes_checked: checked,
        decoded_bytes: decoded,
        zbic_segments: zbic,
    })
}
