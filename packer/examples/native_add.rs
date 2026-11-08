//! Increment 3 validation/measurement driver, not a production CLI.
use nxformat_packer::native::{self, InventoryMode, InventoryOptions, PlannedSnapshot};
use nxformat_packer::*;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
#[path = "support/native_verify.rs"]
mod native_verify;

fn native_options() -> InventoryOptions {
    InventoryOptions {
        mode: InventoryMode::Strict,
        ..InventoryOptions::default()
    }
}
fn inventory(paths: &[PathBuf]) -> Result<native::Inventory, Box<dyn std::error::Error>> {
    let inv = native::inventory(paths, &native_options())?;
    if inv.error_count() != 0 || !inv.plan().blocked.is_empty() {
        return Err("invalid native inventory".into());
    }
    Ok(inv)
}
fn selected(s: &PlannedSnapshot, a: &ApplicationSummary) -> bool {
    a.snapshots.iter().any(|v| {
        v.snapshot_type == s.snapshot_type
            && v.numeric_version == s.numeric_version
            && v.content_id == s.content_id
    })
}
fn number(s: &str) -> u64 {
    u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()
}

// Build a standalone plaintext update/DLC test source using bounded copies.
// This is fixture setup, outside the measured add transaction. No base units
// are copied. Real originals are never renamed, modified or redistributed.
fn isolate(
    destination: &Path,
    source: &native::SourceReport,
    meta: &native::ResolvedMeta,
) -> std::io::Result<()> {
    let units: Vec<_> = meta.units.iter().map(|&i| &source.units[i]).collect();
    let mut strings = Vec::new();
    let mut names = Vec::new();
    for u in &units {
        names.push(strings.len() as u32);
        strings.extend_from_slice(u.name.as_bytes());
        strings.push(0);
    }
    let table_size = 16 + units.len() * 24;
    strings.resize(
        (table_size + strings.len()).div_ceil(32) * 32 - table_size,
        0,
    );
    let mut header = vec![0u8; table_size];
    header[..4].copy_from_slice(b"PFS0");
    header[4..8].copy_from_slice(&(units.len() as u32).to_le_bytes());
    header[8..12].copy_from_slice(&(strings.len() as u32).to_le_bytes());
    let mut at = 0u64;
    for (i, u) in units.iter().enumerate() {
        let e = 16 + i * 24;
        header[e..e + 8].copy_from_slice(&at.to_le_bytes());
        header[e + 8..e + 16].copy_from_slice(&u.size.to_le_bytes());
        header[e + 16..e + 20].copy_from_slice(&names[i].to_le_bytes());
        at += u.size;
    }
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    out.write_all(&header)?;
    out.write_all(&strings)?;
    let mut input = File::open(&source.path)?;
    for u in units {
        input.seek(SeekFrom::Start(u.offset))?;
        if std::io::copy(&mut (&mut input).take(u.size), &mut out)? != u.size {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }
    }
    out.sync_all()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let options = WriterOptions::default();
    match args.first().map(String::as_str) {
        Some("prepare") => {
            let root = Path::new(&args[1]);
            fs::create_dir_all(root)?;
            let paths: Vec<_> = args[2..].iter().map(PathBuf::from).collect();
            if paths.iter().any(|p| {
                p.canonicalize()
                    .ok()
                    .and_then(|p| p.parent().map(Path::to_owned))
                    == root.canonicalize().ok()
            }) {
                return Err(
                    "keep original native sources outside the setup output directory".into(),
                );
            }
            let inv = inventory(&paths)?;
            let plan = inv.plan();
            if plan.applications.len() != 1 {
                return Err("driver requires one application".into());
            }
            let app = &plan.applications[0];
            let input =
                app.to_application_input(&|s| s.snapshot_type == SnapshotType::Base, &|_| None)?;
            write_archive(
                ArchiveInput::new(vec![input]),
                root.join("base.nx"),
                &options,
            )?;
            let mut jobs = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(root.join("jobs.tsv"))?;
            for (i, s) in app
                .snapshots
                .iter()
                .filter(|s| {
                    s.snapshot_type != SnapshotType::Base
                        && (s.snapshot_type == SnapshotType::Dlc || s.program_id_offset == Some(0))
                })
                .enumerate()
            {
                let (source, meta) = inv
                    .sources
                    .iter()
                    .find_map(|source| {
                        source
                            .metas
                            .iter()
                            .find(|m| m.meta.meta_content_id == s.content_id)
                            .map(|m| (source, m))
                    })
                    .ok_or("native meta absent")?;
                let path = root.join(format!("addition-{i}.dnsp"));
                isolate(&path, source, meta)?;
                writeln!(
                    jobs,
                    "{}\t{:016x}\t{}",
                    path.display(),
                    s.title_id,
                    s.numeric_version
                )?;
            }
            println!(
                "prepared {} native additions",
                app.snapshots
                    .iter()
                    .filter(|s| s.snapshot_type != SnapshotType::Base
                        && (s.snapshot_type == SnapshotType::Dlc || s.program_id_offset == Some(0)))
                    .count()
            );
        }
        Some("add") => {
            let request = NativeAddRequest::new(&args[3], number(&args[4]), args[5].parse()?);
            let stats = add_native_content(
                &args[1],
                &args[2],
                &request,
                &native_options(),
                &options,
                IntegrityPolicy::Strict,
            )?;
            println!("{stats:?}");
            println!(
                "METRIC,add,{:.9},{},{},{},{},{},{},{},{},{},{},{}",
                stats.repack.elapsed.as_secs_f64(),
                stats.repack.archive_bytes,
                stats.repack.peak_temporary_disk_bytes,
                stats.accounted_memory_bytes,
                stats.new_native_bytes_staged,
                stats.repack.added_logical_bytes_considered_for_compression,
                stats.base_native_logical_bytes_read,
                stats.base_native_stored_bytes_read,
                stats.new_source_inspection_bytes_read + stats.new_source_equivalence_bytes_read,
                stats.inherited_file_bytes,
                stats.repack.source_payload_bytes_verified,
                stats.repack.retained_stored_bytes_copied
            );
        }
        Some("rebuild") => {
            let start = Instant::now();
            let before = inspect_archive(&args[1], &options, IntegrityPolicy::Strict)?;
            let title = number(&args[3]);
            let version: u32 = args[4].parse()?;
            let paths: Vec<_> = args[5..].iter().map(PathBuf::from).collect();
            let inv = inventory(&paths)?;
            let plan = inv.plan();
            let mut applications = Vec::new();
            for app in &plan.applications {
                let kept = before
                    .applications
                    .iter()
                    .find(|a| a.application_id == app.application_id && a.platform == app.platform)
                    .ok_or("application absent")?;
                let input = app.to_application_input(
                    &|s| selected(s, kept) || (s.title_id == title && s.numeric_version == version),
                    &|r| {
                        app.snapshots
                            .iter()
                            .find(|s| s.reference(app.platform, app.application_id) == *r)
                            .and_then(|s| {
                                kept.snapshots.iter().find(|v| {
                                    v.snapshot_type == s.snapshot_type
                                        && v.program_id == s.program_id
                                        && v.numeric_version == s.numeric_version
                                        && v.content_id == s.content_id
                                })
                            })
                            .map(|s| s.identity)
                    },
                )?;
                applications.push(input);
            }
            let stats = write_archive(ArchiveInput::new(applications), &args[2], &options)?;
            inspect_archive(&args[2], &options, IntegrityPolicy::Strict)?;
            println!("{stats:?}");
            println!(
                "METRIC,rebuild,{:.9},{},{},{},{},{},0,0,{},0,0,0",
                start.elapsed().as_secs_f64(),
                stats.archive_bytes,
                stats.peak_temporary_disk_bytes,
                stats.accounted_memory_bytes,
                stats.input_bytes,
                stats.staged_logical_block_bytes,
                inv.stats.bytes_read
            );
        }
        Some("verify") => {
            let paths: Vec<_> = args[2..].iter().map(PathBuf::from).collect();
            native_verify::verify(Path::new(&args[1]), &paths, &options)?;
        }
        Some("synthetic") => {
            use native::synthetic::*;
            let root = Path::new(&args[1]);
            fs::create_dir_all(root)?;
            const APP: u64 = 0x0100_0000_0001_0000;
            let mut base = TitleSpec::application(APP, 0, 1);
            base.program.as_mut().unwrap().romfs_bytes = 16 * 1024 * 1024;
            fs::write(root.join("base.nsp"), dnsp(&title_entries(&base)))?;
            for version in 1..=3 {
                let mut s = TitleSpec::patch(APP, version, 10 + version as u8);
                s.program.as_mut().unwrap().romfs_bytes = 2 * 1024 * 1024;
                if version == 2 {
                    s.program.as_mut().unwrap().bktr = false;
                }
                if version == 3 {
                    s.program = None;
                    s.control = None;
                    s.data = Some(vec![0x7a; 1234567]);
                }
                fs::write(
                    root.join(format!("update-{version}.nsp")),
                    dnsp(&title_entries(&s)),
                )?;
            }
            for index in 1..=2 {
                let mut s = TitleSpec::add_on(APP, APP + 0x1000 + index, 0, 50 + index as u8);
                if index == 2 {
                    s.required_application_version = 2;
                }
                fs::write(
                    root.join(format!("dlc-{index}.nsp")),
                    dnsp(&title_entries(&s)),
                )?;
            }
        }
        _ => return Err("native_add prepare|add|rebuild|verify|synthetic ...".into()),
    }
    Ok(())
}
