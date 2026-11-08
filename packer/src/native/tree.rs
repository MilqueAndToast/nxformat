//! Extracted ExeFS/RomFS trees (`exefs/main.npdm` and/or `romfs/`).
//!
//! An extracted tree carries an NPDM but no CNMT or Control metadata, so its
//! snapshot type, numeric version and platform generation cannot be
//! established from native records. The inventory reports it and refuses to
//! plan it; directory names are recorded only as hints.

use super::container::{u32_at, u64_at};
use super::{NativeError, TreeSummary};
use std::fs;
use std::path::Path;

/// A directory with `exefs/main.npdm` or a `romfs/` directory. Such trees are
/// reported as one source and never walked file by file.
pub(crate) fn is_extracted_tree(dir: &Path) -> bool {
    dir.join("exefs").is_dir() || dir.join("romfs").is_dir()
}

pub(crate) fn summarize_tree(dir: &Path) -> Result<TreeSummary, NativeError> {
    let has_romfs = dir.join("romfs").is_dir();
    let npdm_path = dir.join("exefs").join("main.npdm");
    let mut exefs_files = Vec::new();
    let mut nso_build_ids = Vec::new();
    if dir.join("exefs").is_dir() {
        for entry in fs::read_dir(dir.join("exefs"))? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| NativeError::invalid("non-UTF8 ExeFS filename"))?;
                if let Some(id) = read_nso_build_id(&entry.path())? {
                    nso_build_ids.push((name.clone(), id));
                }
                exefs_files.push(name);
            }
        }
    }
    exefs_files.sort();
    nso_build_ids.sort();
    let main_build_id = nso_build_ids
        .iter()
        .find(|(name, _)| name == "main")
        .map(|(_, id)| *id);
    if !npdm_path.is_file() {
        return Ok(TreeSummary {
            program_id: None,
            npdm_name: None,
            npdm_flags: None,
            main_build_id,
            exefs_files,
            nso_build_ids,
            has_romfs,
        });
    }
    let len = fs::metadata(&npdm_path)?.len();
    if len > 1024 * 1024 {
        return Err(NativeError::limit("main.npdm exceeds 1 MiB"));
    }
    let npdm = fs::read(&npdm_path)?;
    if npdm.len() < 0x80 || &npdm[..4] != b"META" {
        return Err(NativeError::invalid("exefs/main.npdm lacks the META magic"));
    }
    let aci_off = u32_at(&npdm, 0x70) as usize;
    let aci_size = u32_at(&npdm, 0x74) as usize;
    if aci_size < 0x20
        || aci_off
            .checked_add(aci_size)
            .map_or(true, |e| e > npdm.len())
    {
        return Err(NativeError::invalid("NPDM ACI0 range is invalid"));
    }
    let aci = &npdm[aci_off..aci_off + aci_size];
    if &aci[..4] != b"ACI0" {
        return Err(NativeError::invalid("NPDM ACI0 magic is missing"));
    }
    let name_end = npdm[0x20..0x30].iter().position(|&b| b == 0).unwrap_or(16);
    Ok(TreeSummary {
        program_id: Some(u64_at(aci, 0x10)),
        npdm_name: std::str::from_utf8(&npdm[0x20..0x20 + name_end])
            .ok()
            .map(str::to_owned),
        npdm_flags: Some(npdm[0x0C]),
        main_build_id,
        exefs_files,
        nso_build_ids,
        has_romfs,
    })
}

fn read_nso_build_id(path: &Path) -> Result<Option<[u8; 32]>, NativeError> {
    use std::io::Read;
    let Ok(mut file) = fs::File::open(path) else {
        return Ok(None);
    };
    let mut h = [0u8; 0x60];
    if file.read_exact(&mut h).is_err() || &h[..4] != b"NSO0" {
        return Ok(None);
    }
    let mut id = [0u8; 32];
    id.copy_from_slice(&h[0x40..0x60]);
    Ok(Some(id))
}
