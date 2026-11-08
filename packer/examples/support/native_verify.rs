//! Independent byte comparison using the Rust reader's authenticated references.
use nxformat_packer::native::{self, InventoryMode, InventoryOptions};
use nxformat_packer::*;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

pub fn verify(
    path: &Path,
    sources: &[PathBuf],
    options: &WriterOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let actual = inspect_archive(path, options, IntegrityPolicy::Strict)?;
    let native = InventoryOptions {
        mode: InventoryMode::Strict,
        ..InventoryOptions::default()
    };
    let inventory = native::inventory(sources, &native)?;
    let plan = inventory.plan();
    if inventory.error_count() != 0 || !plan.blocked.is_empty() {
        return Err("invalid native expected inventory".into());
    }
    let mut expected = File::create(path.with_extension("expected.tsv"))?;
    let mut selections = File::create(path.with_extension("selection.tsv"))?;
    let mut packed = File::open(path)?;
    let mut decoder = zstd::bulk::Decompressor::new()?;
    decoder.set_parameter(zstd::zstd_safe::DParameter::WindowLogMax(20))?;
    let mut index = 0;
    let mut count = 0;
    for app in &actual.applications {
        let native_app = plan
            .applications
            .iter()
            .find(|a| a.platform == app.platform && a.application_id == app.application_id)
            .ok_or("native app missing")?;
        for snapshot in &app.snapshots {
            let native_snapshot = native_app
                .snapshots
                .iter()
                .find(|s| {
                    s.program_id == snapshot.program_id
                        && s.snapshot_type == snapshot.snapshot_type
                        && s.numeric_version == snapshot.numeric_version
                        && s.content_id == snapshot.content_id
                })
                .ok_or("native Program/version missing")?;
            if native_snapshot.build_id != snapshot.build_id {
                return Err("native Build ID disagreement".into());
            }
            let expected_nacp = native_snapshot.control.as_ref().map(|c| c.nacp_digest);
            if snapshot.nacp_digest != expected_nacp {
                return Err("per-Program native NACP association disagreement".into());
            }
            let nacp = expected_nacp
                .map(|b| b.iter().map(|v| format!("{v:02x}")).collect::<String>())
                .unwrap_or_else(|| "-".into());
            let program = snapshot
                .program_id
                .map(|p| format!("{p:016x}"))
                .unwrap_or_else(|| "-".into());
            let build = snapshot
                .build_id
                .map(|b| b.iter().map(|v| format!("{v:02x}")).collect::<String>())
                .unwrap_or_else(|| "-".into());
            writeln!(selections, "{index}\t{program}\t{build}\t{nacp}")?;
            let references =
                inspect_snapshot_files(path, snapshot.identity, options, IntegrityPolicy::Fast)?;
            if references.len() != native_snapshot.files.len() {
                return Err("native file count disagreement".into());
            }
            for file in native_snapshot.files.iter() {
                writeln!(
                    expected,
                    "{index}\t{}\t{}\t{}\t{}",
                    file.name,
                    file.source.display(),
                    file.offset,
                    file.size
                )?;
                let reference = references
                    .iter()
                    .find(|r| r.path == file.name)
                    .ok_or("native file missing")?;
                if reference.size != file.size {
                    return Err("native file size disagreement".into());
                }
                let mut original = File::open(&file.source)?;
                original.seek(SeekFrom::Start(file.offset))?;
                let mut done = 0;
                for block in &reference.blocks {
                    let mut frame = vec![0u8; block.stored_size as usize];
                    packed.seek(SeekFrom::Start(block.offset))?;
                    packed.read_exact(&mut frame)?;
                    let decoded = if block.codec == 0 {
                        frame
                    } else {
                        if zstd::zstd_safe::find_frame_compressed_size(&frame).ok()
                            != Some(frame.len())
                        {
                            return Err("trailing frame bytes".into());
                        }
                        decoder.decompress(&frame, block.logical_size as usize)?
                    };
                    if decoded.len() != block.logical_size as usize {
                        return Err("decoded size disagreement".into());
                    }
                    let mut native = vec![0u8; decoded.len()];
                    original.read_exact(&mut native)?;
                    if decoded != native {
                        return Err("Rust original native bytes differ".into());
                    }
                    done += decoded.len() as u64;
                }
                if done != file.size {
                    return Err("native coverage disagreement".into());
                }
                count += 1;
            }
            index += 1;
        }
    }
    println!("PASS Rust native byte comparisons: {count} files; bounded frame reads, explicit Program/Build selection");
    Ok(())
}
