//! Read-only, seekable decryption of Switch NCA3 content inside NSP/XCI.
//! Keys are supplied locally. No keys or decrypted temporary files are written.
//! Binary layouts: Switchbrew NCA/Ticket documentation and the public
//! Atmosphere filesystem format definitions. This module does not verify Nintendo signatures.
use crate::{hash::sha256, source_file::SourceReader};
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes128;
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    sync::{Arc, OnceLock, RwLock},
};
use zeroize::Zeroizing;

fn bad(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn u32le(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(b[p..p + 4].try_into().unwrap())
}
fn u64le(b: &[u8], p: usize) -> u64 {
    u64::from_le_bytes(b[p..p + 8].try_into().unwrap())
}
fn range(start: u64, size: u64, limit: u64) -> io::Result<u64> {
    start
        .checked_add(size)
        .filter(|end| *end <= limit)
        .ok_or_else(|| bad("package range out of bounds"))
}
fn bytes(file: &mut File, start: u64, size: usize) -> io::Result<Vec<u8>> {
    let mut data = vec![0; size];
    file.seek(SeekFrom::Start(start))?;
    file.read_exact(&mut data)?;
    Ok(data)
}
fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

struct Keys {
    values: HashMap<String, Zeroizing<Vec<u8>>>,
}
impl Keys {
    fn load_file(&mut self, path: &Path, titles: bool) -> io::Result<()> {
        let file = File::open(path)?;
        if file.metadata()?.len() > 4 * 1024 * 1024 {
            return Err(bad("key file exceeds 4 MiB"));
        }
        let mut text = Zeroizing::new(String::new());
        file.take(4 * 1024 * 1024 + 1).read_to_string(&mut text)?;
        for (line, raw) in text.lines().enumerate() {
            let raw = raw
                .split('#')
                .next()
                .unwrap()
                .trim()
                .trim_start_matches('\u{feff}');
            if raw.is_empty() {
                continue;
            }
            let Some((name, value)) = raw.split_once('=') else {
                return Err(bad(format!("invalid key-file syntax on line {}", line + 1)));
            };
            let name = name.trim().to_ascii_lowercase();
            let relevant = if titles {
                name.len() == 32 && unhex(&name).is_some()
            } else {
                name == "header_key"
                    || name.starts_with("key_area_key_")
                    || name.starts_with("titlekek_")
                    || name.starts_with("master_key_")
                    || matches!(
                        name.as_str(),
                        "aes_kek_generation_source" | "aes_key_generation_source"
                    )
            };
            if !relevant {
                continue;
            }
            let value = Zeroizing::new(
                unhex(value.trim())
                    .ok_or_else(|| bad(format!("invalid hex on key-file line {}", line + 1)))?,
            );
            let expected = if name == "header_key" { 32 } else { 16 };
            if value.len() != expected {
                return Err(bad(format!("incorrect key length on line {}", line + 1)));
            }
            if let Some(old) = self.values.get(&name) {
                if **old != *value {
                    return Err(bad(format!("conflicting key on line {}", line + 1)));
                }
            }
            self.values.insert(name, value);
        }
        Ok(())
    }
    fn key(&self, name: &str) -> io::Result<[u8; 16]> {
        self.values
            .get(name)
            .and_then(|v| v.as_slice().try_into().ok())
            .ok_or_else(|| bad(format!("missing {name} in the selected key files")))
    }
    fn titlekek(&self, revision: u8) -> io::Result<[u8; 16]> {
        self.key(&format!("titlekek_{revision:02x}")).or_else(|_| {
            Ok(ecb(
                &self.key(&format!("master_key_{revision:02x}"))?,
                self.key("titlekek_source")?,
            ))
        })
    }
    fn area(&self, index: u8, revision: u8) -> io::Result<[u8; 16]> {
        let family = match index {
            0 => "application",
            1 => "ocean",
            2 => "system",
            _ => return Err(bad("unknown NCA key-area index")),
        };
        self.key(&format!("key_area_key_{family}_{revision:02x}"))
            .or_else(|_| {
                let master = self.key(&format!("master_key_{revision:02x}"))?;
                let kek = ecb(&master, self.key("aes_kek_generation_source")?);
                let source = ecb(&kek, self.key(&format!("key_area_key_{family}_source"))?);
                Ok(ecb(&source, self.key("aes_key_generation_source")?))
            })
    }
}
fn config() -> &'static RwLock<Option<Arc<Keys>>> {
    static KEYS: OnceLock<RwLock<Option<Arc<Keys>>>> = OnceLock::new();
    KEYS.get_or_init(Default::default)
}
/// Load prod.keys and optionally title.keys (encrypted title keys keyed by rights ID).
/// If no title file is specified, an adjacent title.keys is used when present.
/// A failed load leaves the previous configuration intact. Only change keys while idle.
pub fn load_keys(prod: &Path, titles: Option<&Path>) -> io::Result<()> {
    let mut keys = Keys {
        values: HashMap::new(),
    };
    keys.load_file(prod, false)?;
    if !keys.values.contains_key("header_key") {
        return Err(bad("prod.keys has no header_key"));
    }
    let adjacent = prod.with_file_name("title.keys");
    if let Some(path) = titles {
        keys.load_file(path, true)?;
    } else if adjacent.is_file() {
        keys.load_file(&adjacent, true)?;
    }
    *config().write().unwrap() = Some(Arc::new(keys));
    Ok(())
}
pub fn keys_loaded() -> bool {
    config().read().unwrap().is_some()
}

fn ecb(key: &[u8; 16], data: [u8; 16]) -> [u8; 16] {
    let mut block = data.into();
    Aes128::new(key.into()).decrypt_block(&mut block);
    block.into()
}
fn xts_decrypt(data: &mut [u8], key: &[u8], first_sector: u64) {
    let data_cipher = Aes128::new_from_slice(&key[..16]).unwrap();
    let tweak_cipher = Aes128::new_from_slice(&key[16..32]).unwrap();
    for (i, sector) in data.chunks_mut(0x200).enumerate() {
        let mut tweak = [0; 16];
        // Nintendo uses a big-endian 128-bit sector number for the XTS tweak.
        tweak[8..].copy_from_slice(&(first_sector + i as u64).to_be_bytes());
        tweak_cipher.encrypt_block((&mut tweak).into());
        for block in sector.chunks_exact_mut(16) {
            for j in 0..16 {
                block[j] ^= tweak[j];
            }
            data_cipher.decrypt_block(block.into());
            for j in 0..16 {
                block[j] ^= tweak[j];
            }
            let mut carry = 0;
            for b in &mut tweak {
                let next = *b >> 7;
                *b = (*b << 1) | carry;
                carry = next;
            }
            if carry != 0 {
                tweak[0] ^= 0x87;
            }
        }
    }
}
fn ctr(data: &mut [u8], cipher: &Aes128, upper: [u8; 8], offset: u64) {
    let mut done = 0;
    while done < data.len() {
        let at = offset + done as u64;
        let mut block = [0; 16];
        block[..8].copy_from_slice(&upper);
        block[8..].copy_from_slice(&(at / 16).to_be_bytes());
        cipher.encrypt_block((&mut block).into());
        let skip = (at % 16) as usize;
        let count = (16 - skip).min(data.len() - done);
        for i in 0..count {
            data[done + i] ^= block[skip + i];
        }
        done += count;
    }
}

struct Entry {
    start: u64,
    size: u64,
    name: String,
}
fn partition(file: &mut File, start: u64, size: u64, hfs: bool) -> io::Result<Vec<Entry>> {
    let end = range(start, size, file.metadata()?.len())?;
    range(start, 16, end)?;
    let head = bytes(file, start, 16)?;
    if &head[..4] != if hfs { b"HFS0" } else { b"PFS0" } {
        return Err(bad("invalid package partition magic"));
    }
    let count = u32le(&head, 4) as usize;
    let names = u32le(&head, 8) as usize;
    if count > 65536 || names > 4 * 1024 * 1024 {
        return Err(bad("package table exceeds limits"));
    }
    let stride = if hfs { 64 } else { 24 };
    let table_size = count * stride + names;
    let data = range(start + 16, table_size as u64, end)?;
    let table = bytes(file, start + 16, table_size)?;
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let p = i * stride;
        let at = range(data, u64le(&table, p), end)?;
        let size = u64le(&table, p + 8);
        range(at, size, end)?;
        let n = u32le(&table, p + 16) as usize;
        if n >= names {
            return Err(bad("package name outside string table"));
        }
        let s = &table[count * stride + n..];
        let stop = s
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| bad("unterminated package name"))?;
        let name = std::str::from_utf8(&s[..stop])
            .map_err(|_| bad("non-UTF8 package name"))?
            .to_owned();
        entries.push(Entry {
            start: at,
            size,
            name,
        });
    }
    entries.sort_by_key(|e| e.start);
    for pair in entries.windows(2) {
        if pair[0].start + pair[0].size > pair[1].start {
            return Err(bad("overlapping package entries"));
        }
    }
    Ok(entries)
}

enum Transform {
    Replace(Vec<u8>),
    Ctr {
        cipher: Aes128,
        upper: [u8; 8],
        nca_start: u64,
    },
}
struct Span {
    start: u64,
    end: u64,
    transform: Transform,
}
pub(crate) struct Decrypted {
    file: File,
    spans: Vec<Span>,
}
impl SourceReader for Decrypted {
    fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(buffer)?;
        let end = offset
            .checked_add(buffer.len() as u64)
            .ok_or_else(|| bad("read overflow"))?;
        // Spans do not overlap; binary search avoids scanning every DLC/patch range per read.
        let first = self.spans.partition_point(|s| s.end <= offset);
        for span in self.spans[first..].iter().take_while(|s| s.start < end) {
            let a = offset.max(span.start);
            let b = end.min(span.end);
            let out = &mut buffer[(a - offset) as usize..(b - offset) as usize];
            match &span.transform {
                Transform::Replace(data) => {
                    out.copy_from_slice(&data[(a - span.start) as usize..(b - span.start) as usize])
                }
                Transform::Ctr {
                    cipher,
                    upper,
                    nca_start,
                } => ctr(out, cipher, *upper, a - nca_start),
            }
        }
        Ok(())
    }
}

/// Returns None when the file is not an encrypted-package candidate or no keys are configured.
pub(crate) fn open(path: &Path) -> io::Result<Option<(u64, Decrypted)>> {
    let Some(keys) = config().read().unwrap().clone() else {
        return Ok(None);
    };
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    if !["nsp", "xci", "dnsp", "dxci"]
        .iter()
        .any(|e| extension.eq_ignore_ascii_case(e))
    {
        return Ok(None);
    }
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    if size < 16 {
        return Err(bad("package is too small"));
    }
    let head = bytes(&mut file, 0, size.min(0x200) as usize)?;
    let mut spans = Vec::new();
    let entries = if &head[..4] == b"PFS0" {
        partition(&mut file, 0, size, false)?
    } else if head.len() >= 0x138 && &head[0x100..0x104] == b"HEAD" {
        let root = u64le(&head, 0x130);
        if root >= size {
            return Err(bad("gamecard root is out of bounds"));
        }
        let partitions = partition(&mut file, root, size - root, true)?;
        let mut entries = Vec::new();
        for part in partitions {
            // The gamecard update partition is console firmware, not game updates.
            // Upstream keeps it as an opaque resource or omits it with --omit-firmware.
            if part.name != "update" {
                entries.extend(partition(&mut file, part.start, part.size, true)?);
            }
        }
        spans.push(Span {
            start: 0x100,
            end: 0x104,
            transform: Transform::Replace(b"DXCI".to_vec()),
        });
        entries
    } else {
        return Ok(None);
    };
    let mut tickets: HashMap<[u8; 16], [u8; 16]> = HashMap::new();
    for entry in &entries {
        if !entry.name.ends_with(".tik") {
            continue;
        }
        if entry.size > 0x10000 || entry.size < 4 {
            return Err(bad("ticket size outside limits"));
        }
        let ticket = bytes(&mut file, entry.start, entry.size as usize)?;
        let sig = u32le(&ticket, 0);
        let p = match sig {
            0x10000 | 0x10003 => 0x240,
            0x10001 | 0x10004 => 0x140,
            0x10002 | 0x10005 => 0x80,
            _ => return Err(bad("unsupported ticket signature type")),
        };
        if ticket.len() < p + 0x180 {
            return Err(bad("truncated ticket"));
        }
        let id: [u8; 16] = ticket[p + 0x160..p + 0x170].try_into().unwrap();
        if ticket[p + 0x141] == 0 {
            let key = ticket[p + 0x40..p + 0x50].try_into().unwrap();
            if let Some(old) = tickets.insert(id, key) {
                if old != key {
                    return Err(bad("conflicting common tickets"));
                }
            }
        }
    }
    for entry in entries {
        if entry.name.ends_with(".ncz") {
            return Err(bad(
                "NCZ compression is unsupported; decompress the package first",
            ));
        }
        if !entry.name.ends_with(".nca") {
            continue;
        }
        add_nca(&mut file, &entry, &keys, &tickets, &mut spans)
            .map_err(|e| bad(format!("{}: {e}", entry.name)))?;
    }
    spans.sort_by_key(|s| s.start);
    for pair in spans.windows(2) {
        if pair[0].end > pair[1].start {
            return Err(bad("overlapping decryption ranges"));
        }
    }
    Ok(Some((size, Decrypted { file, spans })))
}

fn add_ctr(
    spans: &mut Vec<Span>,
    start: u64,
    end: u64,
    nca_start: u64,
    key: &[u8; 16],
    upper: [u8; 8],
) {
    if start < end {
        spans.push(Span {
            start,
            end,
            transform: Transform::Ctr {
                cipher: Aes128::new(key.into()),
                upper,
                nca_start,
            },
        });
    }
}
fn add_nca(
    file: &mut File,
    entry: &Entry,
    keys: &Keys,
    tickets: &HashMap<[u8; 16], [u8; 16]>,
    spans: &mut Vec<Span>,
) -> io::Result<()> {
    if entry.size < 0xc00 {
        return Err(bad("NCA header truncated"));
    }
    let mut header = bytes(file, entry.start, 0xc00)?;
    if &header[0x200..0x204] == b"DNCA" {
        return Ok(());
    }
    if !matches!(&header[0x200..0x204], b"NCA3" | b"NCA2") {
        xts_decrypt(&mut header, keys.values.get("header_key").unwrap(), 0);
    }
    if &header[0x200..0x204] != b"NCA3" {
        return Err(bad(
            "cannot decrypt NCA3 header: wrong header_key or unsupported NCA version",
        ));
    }
    if u64le(&header, 0x208) != entry.size {
        return Err(bad("NCA size differs from package entry"));
    }
    let revision = header[0x206].max(header[0x220]).saturating_sub(1);
    let rights: [u8; 16] = header[0x230..0x240].try_into().unwrap();
    let mut ctr_key = None;
    for i in 0..4 {
        let start = u32le(&header, 0x240 + i * 16) as u64 * 0x200;
        let end = u32le(&header, 0x244 + i * 16) as u64 * 0x200;
        if start == 0 && end == 0 {
            continue;
        }
        if start < 0xc00 || end <= start {
            return Err(bad("invalid NCA section range"));
        }
        let f = 0x400 + i * 0x200;
        let fs = &header[f..f + 0x200];
        if sha256(fs) != header[0x280 + i * 32..0x2a0 + i * 32] {
            return Err(bad(
                "NCA filesystem header hash mismatch (wrong key or damaged input)",
            ));
        }
        if fs[0..2] != [2, 0] {
            return Err(bad("unsupported filesystem header version"));
        }
        if fs[0x148..0x178].iter().any(|b| *b != 0) {
            return Err(bad("sparse NCA is not supported by this decryption build"));
        }
        if end > entry.size {
            return Err(bad("NCA section extends beyond file"));
        }
        let enc = fs[4];
        if enc != 1 {
            if !matches!(enc, 3..=6) {
                return Err(bad(format!(
                    "unsupported NCA section encryption type {enc}"
                )));
            }
            let key = if let Some(key) = ctr_key {
                key
            } else {
                let key = if rights.iter().any(|b| *b != 0) {
                    let id = rights
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>();
                    let encrypted=keys.values.get(&id).map(|v|v.as_slice().try_into().unwrap()).or_else(||tickets.get(&rights).copied())
                        .ok_or_else(||bad(format!("missing title key for rights ID {id}; a personalized ticket needs a recovered entry in title.keys")))?;
                    ecb(&keys.titlekek(revision)?, encrypted)
                } else {
                    ecb(
                        &keys.area(header[0x207], revision)?,
                        header[0x320..0x330].try_into().unwrap(),
                    )
                };
                ctr_key = Some(key);
                key
            };
            let upper = u64le(fs, 0x140).to_be_bytes();
            let skip = if enc >= 5 { hash_target(fs)? } else { 0 };
            if skip > end - start {
                return Err(bad("hash-layer offset outside section"));
            }
            if enc == 3 || enc == 5 {
                add_ctr(
                    spans,
                    entry.start + start + skip,
                    entry.start + end,
                    entry.start,
                    &key,
                    upper,
                );
            } else {
                extended(
                    file,
                    entry.start,
                    start,
                    end - start,
                    fs,
                    &key,
                    upper,
                    skip,
                    spans,
                )?;
            }
        }
        header[f + 4] = 1;
        let hash = sha256(&header[f..f + 0x200]);
        header[0x280 + i * 32..0x2a0 + i * 32].copy_from_slice(&hash);
    }
    header[0x200..0x204].copy_from_slice(b"DNCA");
    // A decrypted view needs neither encrypted key area nor personalized identifiers.
    header[0x230..0x240].fill(0);
    header[0x300..0x340].fill(0);
    spans.push(Span {
        start: entry.start,
        end: entry.start + 0xc00,
        transform: Transform::Replace(header),
    });
    Ok(())
}
fn hash_target(fs: &[u8]) -> io::Result<u64> {
    match fs[3] {
        2 | 5 => {
            let n = u32le(fs, 0x2c) as usize;
            if n == 0 || n > 5 {
                return Err(bad("invalid SHA layer count"));
            }
            Ok(u64le(fs, 0x30 + (n - 1) * 16))
        }
        3 | 6 => {
            let n = u32le(fs, 0x14) as usize;
            if n < 2 || n > 7 {
                return Err(bad("invalid integrity layer count"));
            }
            Ok(u64le(fs, 0x18 + (n - 2) * 24))
        }
        _ => Err(bad("unsupported skip-layer-hash layout")),
    }
}
fn extended(
    file: &mut File,
    nca: u64,
    start: u64,
    size: u64,
    fs: &[u8],
    key: &[u8; 16],
    upper: [u8; 8],
    skip: u64,
    spans: &mut Vec<Span>,
) -> io::Result<()> {
    let table_at = u64le(fs, 0x120);
    let table_size = u64le(fs, 0x128);
    if table_size == 0 {
        add_ctr(
            spans,
            nca + start + skip,
            nca + start + size,
            nca,
            key,
            upper,
        );
        return Ok(());
    }
    if &fs[0x130..0x134] != b"BKTR" || u32le(fs, 0x134) != 1 {
        return Err(bad("invalid CTR-EX table header"));
    }
    range(table_at, table_size, size)?;
    if table_size > 64 * 1024 * 1024 {
        return Err(bad("CTR-EX table exceeds 64 MiB"));
    }
    let count = u32le(fs, 0x138) as usize;
    if count == 0 || count > 1_000_000 {
        return Err(bad("CTR-EX entry count outside limits"));
    }
    let blocks = count.div_ceil(1023);
    // Entry sets are 16 KiB with a 16-byte header. Large two-level indexes
    // are deliberately rejected until independently tested.
    if blocks > 2046 {
        return Err(bad("CTR-EX tree is too large"));
    }
    if (blocks + 1) * 0x4000 > table_size as usize {
        return Err(bad("truncated CTR-EX table"));
    }
    let mut data = bytes(file, nca + start + table_at, table_size as usize)?;
    ctr(&mut data, &Aes128::new(key.into()), upper, start + table_at);
    let mut entries = Vec::with_capacity(count);
    let mut previous_end = 0;
    for block in 0..blocks {
        let p = (block + 1) * 0x4000;
        let n = u32le(&data, p + 4) as usize;
        let end = u64le(&data, p + 8);
        if u32le(&data, p) as usize != block
            || n == 0
            || n > 1023
            || entries.len() + n > count
            || end > table_at
        {
            return Err(bad(
                "invalid CTR-EX entry set (wrong title key or damaged input)",
            ));
        }
        for j in 0..n {
            let q = p + 16 + j * 16;
            let offset = u64le(&data, q);
            if (j == 0 && offset != previous_end)
                || offset >= end
                || data[q + 8] > 1
                || data[q + 9..q + 12] != [0, 0, 0]
            {
                return Err(bad("invalid CTR-EX entry"));
            }
            if let Some((last, _, _)) = entries.last() {
                if *last >= offset {
                    return Err(bad("unsorted CTR-EX entries"));
                }
            }
            entries.push((offset, data[q + 8], u32le(&data, q + 12)));
        }
        previous_end = end;
    }
    let indirect = u64le(fs, 0x100);
    let indirect_size = u64le(fs, 0x108);
    let data_end = if indirect_size != 0 {
        table_at.min(indirect)
    } else {
        table_at
    };
    if entries.len() != count || previous_end < data_end {
        return Err(bad("incomplete CTR-EX coverage"));
    }
    // Metadata uses the base counter stream, overriding the per-region stream.
    let mut meta_start = table_at;
    let mut meta_end = table_at + table_size;
    if indirect_size != 0 {
        range(indirect, indirect_size, size)?;
        meta_start = meta_start.min(indirect);
        meta_end = meta_end.max(indirect + indirect_size);
        if fs[5] != 0 {
            let off = u64le(fs, 0x1a0);
            let len = u64le(fs, 0x1a8);
            meta_end = meta_end.max(
                range(off, len, size)?
                    .checked_add(15)
                    .ok_or_else(|| bad("metadata range overflow"))?
                    & !15,
            );
        }
    }
    if meta_end > size {
        return Err(bad("patch metadata outside section"));
    }
    for (i, (at, plain, generation)) in entries.iter().enumerate() {
        let end = entries
            .get(i + 1)
            .map(|v| v.0)
            .unwrap_or(table_at)
            .min(meta_start);
        if *plain == 0 {
            let mut iv = upper;
            iv[4..].copy_from_slice(&generation.to_be_bytes());
            add_ctr(
                spans,
                nca + start + (*at).max(skip),
                nca + start + end,
                nca,
                key,
                iv,
            );
        }
    }
    add_ctr(
        spans,
        nca + start + meta_start.max(skip),
        nca + start + meta_end,
        nca,
        key,
        upper,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(std::path::PathBuf);
    impl Temp {
        fn new(data: &[u8]) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static ID: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "nxformat-decrypt-test-{}-{}",
                std::process::id(),
                ID.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::write(&path, data).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn key_files_are_bounded_redacted_and_reject_conflicts() {
        let mut keys = Keys {
            values: HashMap::new(),
        };
        let sample = Temp::new(
            b"header_key = 000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f\n",
        );
        keys.load_file(&sample.0, false).unwrap();
        assert_eq!(keys.values["header_key"].len(), 32);
        let conflict = Temp::new(
            b"header_key = ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff\n",
        );
        let error = keys.load_file(&conflict.0, false).unwrap_err().to_string();
        assert!(error.contains("conflicting key"));
        assert!(!error.contains("ffff"));
        let bad = Temp::new(b"header_key = SECRET_BAD_HEX\n");
        let error = keys.load_file(&bad.0, false).unwrap_err().to_string();
        assert!(!error.contains("SECRET"));
        assert_eq!(keys.values["header_key"][1], 1);
    }

    #[test]
    fn ctr_ex_legacy_and_new_metadata_boundaries_stream_exact_bytes() {
        for new_boundary in [false, true] {
            for unencrypted_data in [false, true] {
                let start = 0xc00u64;
                let key = [0x55; 16];
                let cipher = Aes128::new((&key).into());
                let upper = [0, 0, 0, 9, 0, 0, 0, 2];
                let mut fs = vec![0; 0x200];
                fs[0x100..0x108].copy_from_slice(&0x4000u64.to_le_bytes());
                fs[0x108..0x110].copy_from_slice(&0x4000u64.to_le_bytes());
                fs[0x120..0x128].copy_from_slice(&0x8000u64.to_le_bytes());
                fs[0x128..0x130].copy_from_slice(&0x8000u64.to_le_bytes());
                fs[0x130..0x134].copy_from_slice(b"BKTR");
                fs[0x134] = 1;
                fs[0x138] = 1;
                let mut plain = vec![0x6b; 0x10000];
                plain[0x8000..].fill(0);
                let p = 0xc000;
                plain[p + 4..p + 8].copy_from_slice(&1u32.to_le_bytes());
                plain[p + 8..p + 16].copy_from_slice(
                    &(if new_boundary { 0x4000u64 } else { 0x8000u64 }).to_le_bytes(),
                );
                plain[p + 24] = u8::from(unencrypted_data);
                plain[p + 28] = 7;
                let mut encrypted = plain.clone();
                if !unencrypted_data {
                    ctr(
                        &mut encrypted[..0x4000],
                        &cipher,
                        [0, 0, 0, 9, 0, 0, 0, 7],
                        start,
                    );
                }
                ctr(&mut encrypted[0x4000..], &cipher, upper, start + 0x4000);
                let mut disk = vec![0xa5; start as usize];
                disk.extend(encrypted);
                let temp = Temp::new(&disk);
                let mut file = File::open(&temp.0).unwrap();
                let mut spans = Vec::new();
                extended(
                    &mut file, 0, start, 0x10000, &fs, &key, upper, 0, &mut spans,
                )
                .unwrap();
                let mut view = Decrypted { file, spans };
                let mut actual = vec![0; 0x10000];
                // Odd read boundaries exercise transitions into metadata and between tables.
                for (i, chunk) in actual.chunks_mut(997).enumerate() {
                    view.read_at(start + i as u64 * 997, chunk).unwrap();
                }
                assert_eq!(actual, plain);
                let mut file = File::open(&temp.0).unwrap();
                fs[0x138] = 2;
                assert!(extended(
                    &mut file,
                    0,
                    start,
                    0x10000,
                    &fs,
                    &key,
                    upper,
                    0,
                    &mut Vec::new()
                )
                .is_err());
            }
        }
    }
    #[test]
    fn openssl_xts_and_ctr_vectors() {
        // Independently generated using OpenSSL via Python cryptography.
        // Nonzero sector catches the NCA-specific big-endian XTS tweak convention.
        let key: Vec<u8> = (0..32).collect();
        let mut data =
            unhex("62b9adbc34e5fe6f3d0c25ec97991f5f17dad916981c79e0f6d715962d9a4585").unwrap();
        xts_decrypt(&mut data, &key, 0x123);
        assert_eq!(data, (0..32).collect::<Vec<u8>>());
        let mut data = vec![0; 32];
        ctr(
            &mut data,
            &Aes128::new_from_slice(&key[..16]).unwrap(),
            [1, 2, 3, 4, 5, 6, 7, 8],
            0x1235,
        );
        assert_eq!(
            data,
            unhex("8a73b5d06cebf885cb7191ffcf07a8cf275ab73beed2564f2d78587c8d47b35e").unwrap()
        );
    }
    #[test]
    fn aes_known_answer_and_unaligned_ctr() {
        // NIST SP 800-38A F.5.1, first block.
        let key: [u8; 16] = unhex("2b7e151628aed2a6abf7158809cf4f3c")
            .unwrap()
            .try_into()
            .unwrap();
        let cipher = Aes128::new((&key).into());
        let upper = unhex("f0f1f2f3f4f5f6f7").unwrap().try_into().unwrap();
        let mut data = unhex("6bc1bee22e409f96e93d7e117393172a").unwrap();
        // The NCA counter has a 64-bit block address; use an equivalent direct AES vector.
        let mut block = unhex("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff").unwrap();
        cipher.encrypt_block(block.as_mut_slice().into());
        for (a, b) in data.iter_mut().zip(block) {
            *a ^= b;
        }
        assert_eq!(data, unhex("874d6191b620e3261bef6864990db6ce").unwrap());
        let mut all = vec![0; 5000];
        ctr(&mut all, &cipher, upper, 0x1230);
        for start in [0, 1, 15, 16, 31, 511, 512, 4095] {
            let mut slice = vec![0; 83];
            ctr(&mut slice, &cipher, upper, 0x1230 + start as u64);
            assert_eq!(slice, all[start..start + 83]);
        }
    }
    #[test]
    fn rejects_overflow_and_bad_hex() {
        assert!(range(u64::MAX, 1, u64::MAX).is_err());
        assert!(range(1, 10, 10).is_err());
        assert!(unhex("aa日").is_none());
        assert!(unhex("a").is_none());
    }
}
