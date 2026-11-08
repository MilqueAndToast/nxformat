//! Bounded positional readers and PFS0/HFS0 partition parsing.

use super::{InventoryOptions, NativeError};
use std::io::{Read, Seek, SeekFrom};
use std::{cell::Cell, rc::Rc};

/// A source file with a known length. Every read is bounds-checked against
/// that length so truncated containers fail as `Truncated`, not as I/O noise.
pub(crate) trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

pub(crate) struct Source<'a> {
    file: Box<dyn ReadSeek + 'a>,
    pub(crate) len: u64,
    pub(crate) bytes_read: u64,
    metadata_budget: Option<Rc<Cell<u64>>>,
    metadata_exhausted: bool,
}

impl<'a> Source<'a> {
    pub(crate) fn new(file: impl ReadSeek + 'a, len: u64) -> Self {
        Self {
            file: Box::new(file),
            len,
            bytes_read: 0,
            metadata_budget: None,
            metadata_exhausted: false,
        }
    }

    pub(crate) fn with_metadata_budget(mut self, budget: Rc<Cell<u64>>) -> Self {
        self.metadata_budget = Some(budget);
        self
    }

    // Bound cumulative retained native facts, not only each individual CNMT.
    // The factor covers projected records/strings and their planner copies.
    pub(crate) fn reserve_metadata(&mut self, bytes: u64) -> Result<(), NativeError> {
        if let Some(budget) = &self.metadata_budget {
            let charge = bytes
                .checked_mul(8)
                .ok_or_else(|| NativeError::limit("native metadata accounting overflow"))?;
            if charge > budget.get() {
                self.metadata_exhausted = true;
                return Err(NativeError::limit(
                    "cumulative native metadata budget exceeded",
                ));
            }
            budget.set(budget.get() - charge);
        }
        Ok(())
    }

    pub(crate) fn check_metadata_budget(&self) -> Result<(), NativeError> {
        if self.metadata_exhausted {
            return Err(NativeError::limit(
                "cumulative native metadata budget exceeded",
            ));
        }
        Ok(())
    }

    pub(crate) fn read_exact_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), NativeError> {
        let end = offset
            .checked_add(buf.len() as u64)
            .ok_or_else(|| NativeError::invalid("read range overflows"))?;
        if end > self.len {
            return Err(NativeError::truncated(format!(
                "read of {} bytes at {offset:#x} exceeds source size {:#x}",
                buf.len(),
                self.len
            )));
        }
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(buf)?;
        self.bytes_read += buf.len() as u64;
        Ok(())
    }

    pub(crate) fn read_vec_at(
        &mut self,
        offset: u64,
        size: u64,
        limit: u64,
        what: &str,
    ) -> Result<Vec<u8>, NativeError> {
        if size > limit {
            return Err(NativeError::limit(format!(
                "{what} is {size} bytes, above the {limit}-byte limit"
            )));
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size as usize)
            .map_err(|_| NativeError::limit(format!("unable to reserve {what}")))?;
        bytes.resize(size as usize, 0);
        self.read_exact_at(offset, &mut bytes)?;
        Ok(bytes)
    }
}

pub(crate) fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
pub(crate) fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
pub(crate) fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PartitionEntry {
    pub(crate) name: String,
    /// Absolute offset in the source file.
    pub(crate) offset: u64,
    pub(crate) size: u64,
}

/// Parse a PFS0 (24-byte entries) or HFS0 (64-byte entries) partition that
/// starts at `base` and must end at or before `end`.
pub(crate) fn parse_partition(
    src: &mut Source,
    base: u64,
    end: u64,
    hfs: bool,
    options: &InventoryOptions,
) -> Result<Vec<PartitionEntry>, NativeError> {
    let magic: &[u8; 4] = if hfs { b"HFS0" } else { b"PFS0" };
    let entry_size = if hfs { 64u64 } else { 24u64 };
    if end > src.len {
        return Err(NativeError::truncated("partition extends past the source"));
    }
    let mut head = [0u8; 16];
    if base.checked_add(16).map_or(true, |e| e > end) {
        return Err(NativeError::truncated("partition header is truncated"));
    }
    src.read_exact_at(base, &mut head)?;
    if &head[..4] != magic {
        return Err(NativeError::invalid(format!(
            "expected {} partition magic",
            String::from_utf8_lossy(magic)
        )));
    }
    let count = u32_at(&head, 4);
    let string_size = u32_at(&head, 8) as u64;
    if count > options.max_partition_entries {
        return Err(NativeError::limit(format!(
            "partition lists {count} entries, above the configured limit"
        )));
    }
    if string_size > options.max_string_table_bytes {
        return Err(NativeError::limit(
            "partition string table exceeds the configured limit",
        ));
    }
    let table_size = count as u64 * entry_size;
    let data_start = base + 16 + table_size + string_size;
    if data_start > end {
        return Err(NativeError::truncated("partition tables are truncated"));
    }
    let table = src.read_vec_at(base + 16, table_size, u64::MAX, "partition entries")?;
    let strings = src.read_vec_at(
        base + 16 + table_size,
        string_size,
        u64::MAX,
        "partition string table",
    )?;
    let mut entries = Vec::with_capacity(count as usize);
    let mut ranges = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let e = &table[i * entry_size as usize..(i + 1) * entry_size as usize];
        let rel = u64_at(e, 0);
        let size = u64_at(e, 8);
        let name_off = u32_at(e, 16) as usize;
        if name_off >= strings.len() {
            return Err(NativeError::invalid("partition entry name is out of range"));
        }
        let name_end = strings[name_off..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| name_off + p)
            .ok_or_else(|| NativeError::invalid("partition entry name is unterminated"))?;
        let name = std::str::from_utf8(&strings[name_off..name_end])
            .map_err(|_| NativeError::invalid("partition entry name is not UTF-8"))?;
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains('/')
            || name.contains('\\')
        {
            return Err(NativeError::invalid("unsafe partition entry name"));
        }
        let offset = data_start
            .checked_add(rel)
            .ok_or_else(|| NativeError::invalid("partition entry offset overflows"))?;
        let entry_end = offset
            .checked_add(size)
            .ok_or_else(|| NativeError::invalid("partition entry size overflows"))?;
        if entry_end > end {
            return Err(NativeError::truncated(format!(
                "partition entry {name} extends past its container"
            )));
        }
        if entries.iter().any(|p: &PartitionEntry| p.name == name) {
            return Err(NativeError::invalid(format!(
                "duplicate partition entry {name}"
            )));
        }
        ranges.push((offset, entry_end));
        entries.push(PartitionEntry {
            name: name.to_owned(),
            offset,
            size,
        });
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|w| w[0].1 > w[1].0) {
        return Err(NativeError::invalid("partition entries overlap"));
    }
    Ok(entries)
}
