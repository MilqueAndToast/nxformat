//! Reproducible synthetic conformance driver and private local conversion tool.
use nxformat_packer::conversion::convert_inventory_profiled;
use nxformat_packer::native::{inventory, InventoryMode, InventoryOptions};
use nxformat_packer::*;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn writer_details(phase: &str, s: &WriterStats) {
    println!(
        "WRITER_DETAIL,{phase},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
        s.elapsed.as_nanos(),
        s.input_read_time.as_nanos(),
        s.input_hash_time.as_nanos(),
        s.global_dedup_time.as_nanos(),
        s.compression_time.as_nanos(),
        s.block_staging_write_time.as_nanos(),
        s.archive_write_time.as_nanos(),
        s.output_verification_time.as_nanos(),
        s.compression_stage_time.as_nanos(),
        s.compression_staging_read_bytes,
        s.compression_queue_peak_bytes,
        s.accounted_memory_bytes,
        s.peak_temporary_disk_bytes,
        s.archive_bytes,
        s.input_bytes,
        s.output_sync_time.as_nanos(),
        s.atomic_publication_time.as_nanos()
    );
}

fn convert(
    destination: &Path,
    roots: &[PathBuf],
    rebuild: bool,
    expected: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let native = InventoryOptions {
        mode: InventoryMode::Strict,
        ..InventoryOptions::default()
    };
    let inv = inventory(roots, &native)?;
    let options = WriterOptions {
        chunk_size_bytes: std::env::var("NX_BENCH_CHUNK_KIB")
            .ok()
            .map_or(Ok(256), |v| v.parse::<u32>())?
            .checked_mul(1024)
            .ok_or("chunk overflow")?,
        compression_workers: std::env::var("NX_BENCH_WORKERS")
            .ok()
            .map_or(Ok(1), |v| v.parse())?,
        block_sizing: if std::env::var_os("NX_BENCH_ADAPTIVE").is_some() {
            BlockSizing::Adaptive
        } else {
            BlockSizing::Fixed
        },
        ..Default::default()
    };
    let profiles = std::env::var_os("NX_BENCH_READ_PROFILE")
        .map(|path| -> Result<_, Box<dyn std::error::Error>> {
            Ok(nxformat_packer::read_profile::CapturedReadProfiles::read(
                fs::File::open(path)?,
            )?)
        })
        .transpose()?;
    let stats = convert_inventory_profiled(
        &inv,
        destination,
        &native,
        IntegrityPolicy::Strict,
        &options,
        profiles.as_ref(),
    )?;
    println!("conversion={stats:?}");
    writer_details("initial", &stats.initial);
    let logical: u64 = inv
        .plan()
        .applications
        .iter()
        .flat_map(|a| &a.snapshots)
        .flat_map(|s| s.files.iter())
        .map(|f| f.size)
        .sum();
    println!(
        "METRIC,conversion,{:.9},{},{},{},{},{},{}",
        stats.elapsed.as_secs_f64(),
        logical,
        fs::metadata(destination)?.len(),
        stats.peak_temporary_disk_bytes,
        stats.initial.deduplicated_blocks
            + stats
                .additions
                .iter()
                .map(|s| s.added_blocks_reused + s.added_input_deduplicated_blocks)
                .sum::<u64>(),
        stats.initial.accounted_memory_bytes,
        inv.stats.bytes_read
    );
    let summary = inspect_archive(destination, &options, IntegrityPolicy::Strict)?;
    let plan = inv.plan();
    if expected {
        let mut manifest = fs::File::create(destination.with_extension("expected.tsv"))?;
        let mut selections = fs::File::create(destination.with_extension("selection.tsv"))?;
        let hex = |bytes: &[u8]| {
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        let mut index = 0;
        for app in &summary.applications {
            let p = plan
                .applications
                .iter()
                .find(|p| p.application_id == app.application_id && p.platform == app.platform)
                .unwrap();
            for snapshot in &app.snapshots {
                writeln!(
                    selections,
                    "{index}\t{:016x}\t{}\t{}\t{}\t{}",
                    app.application_id,
                    snapshot
                        .program_id
                        .map_or("-".to_string(), |id| format!("{id:016x}")),
                    snapshot.numeric_version,
                    hex(&snapshot.identity),
                    snapshot
                        .build_id
                        .map_or("-".to_string(), |build| hex(&build))
                )?;
                let s = p
                    .snapshots
                    .iter()
                    .find(|s| {
                        s.snapshot_type == snapshot.snapshot_type
                            && s.content_id == snapshot.content_id
                            && s.numeric_version == snapshot.numeric_version
                    })
                    .unwrap();
                for f in s.files.iter() {
                    writeln!(
                        manifest,
                        "{index}\t{}\t{}\t{}\t{}",
                        f.name,
                        f.source.display(),
                        f.offset,
                        f.size
                    )?;
                }
                index += 1;
            }
        }
    }
    if !rebuild {
        return Ok(());
    }
    let repacked = destination.with_extension("repacked.nx");
    let repack = repack_archive(
        destination,
        &repacked,
        RepackPlan::new(),
        &options,
        IntegrityPolicy::Strict,
    )?;
    println!("repack={repack:?}");
    assert_eq!(
        summary,
        inspect_archive(&repacked, &options, IntegrityPolicy::Strict)?
    );
    if rebuild {
        let resolve = |r: &nxformat_packer::native::SnapshotRef| {
            summary
                .applications
                .iter()
                .find(|a| a.application_id == r.application_id && a.platform == r.platform)
                .and_then(|a| {
                    a.snapshots.iter().find(|s| {
                        s.snapshot_type == r.snapshot_type
                            && s.program_id == r.program_id
                            && s.numeric_version == r.numeric_version
                    })
                })
                .map(|s| s.identity)
        };
        let input = plan
            .applications
            .iter()
            .map(|a| a.to_application_input(&|_| true, &resolve))
            .collect::<Result<Vec<_>, _>>()?;
        let path = destination.with_extension("rebuilt.nx");
        let rebuilt = write_archive(ArchiveInput::new(input), &path, &options)?;
        assert_eq!(
            summary,
            inspect_archive(&path, &options, IntegrityPolicy::Strict)?
        );
        println!("rebuild={rebuilt:?}");
        writer_details("rebuild", &rebuilt);
        println!(
            "METRIC,rebuild,{:.9},{},{},{},{},{},0",
            rebuilt.elapsed.as_secs_f64(),
            logical,
            rebuilt.archive_bytes,
            rebuilt.peak_temporary_disk_bytes,
            rebuilt.deduplicated_blocks,
            rebuilt.accounted_memory_bytes
        );
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--synthetic") {
        use nxformat_packer::native::synthetic::*;
        const APP: u64 = 0x0100_0000_0001_0000;
        let root = PathBuf::from(args.get(1).ok_or("missing synthetic output directory")?);
        for (name, updates, dlcs) in [
            ("base", 0, 0),
            ("latest", 1, 0),
            ("updates", 3, 0),
            ("dlc", 0, 2),
            ("all", 3, 2),
        ] {
            let dir = root.join(name);
            let source = dir.join("src");
            fs::create_dir_all(&source)?;
            let mut base = TitleSpec::application(APP, 0, 1);
            base.program.as_mut().unwrap().romfs_bytes = 8 * 1024 * 1024;
            base.control.as_mut().unwrap().localized.push((
                3,
                "Titre natif".into(),
                "Editeur natif".into(),
            ));
            fs::write(source.join("base.nsp"), dnsp(&title_entries(&base)))?;
            for v in 1..=updates {
                let mut s = TitleSpec::patch(APP, v, (v + 1) as u8);
                s.control.as_mut().unwrap().title = format!("Native version {v}");
                if v == 2 {
                    s.program = None;
                    s.data = Some(vec![42; 8192]);
                }
                if v == 3 {
                    s.data = Some(vec![43; 8192]);
                    s.program.as_mut().unwrap().bktr = false;
                }
                fs::write(
                    source.join(format!("update{v}.nsp")),
                    dnsp(&title_entries(&s)),
                )?;
            }
            for d in 1..=dlcs {
                let mut dlc = TitleSpec::add_on(APP, APP + 0x1000 + d, 0, (d + 10) as u8);
                if updates >= 2 && d == 2 {
                    dlc.required_application_version = 2;
                }
                fs::write(
                    source.join(format!("dlc{d}.nsp")),
                    dnsp(&title_entries(&dlc)),
                )?;
            }
            println!("scenario={name}");
            convert(&dir.join("game.nx"), &[source], true, true)?;
        }
        // Distinct CNMT applications in one package are representable and must
        // retain their independent Program and version ranges.
        let source = root.join("collection/src");
        fs::create_dir_all(&source)?;
        let mut entries = Vec::new();
        for (app, seed) in [(APP, 1), (APP + 0x2000, 2)] {
            entries.extend(title_entries(&TitleSpec::application(app, 0, seed)));
            entries.extend(title_entries(&TitleSpec::patch(app, 1, seed + 3)));
        }
        fs::write(source.join("collection.nsp"), dnsp(&entries))?;
        println!("scenario=collection");
        convert(&root.join("collection/game.nx"), &[source], true, true)?;
    } else {
        let only = args.first().map(String::as_str) == Some("--convert-only");
        let args = if only { &args[1..] } else { &args[..] };
        let destination = PathBuf::from(
            args.first()
                .ok_or("usage: native_convert DESTINATION SOURCE... | --synthetic DIRECTORY")?,
        );
        let roots = args[1..].iter().map(PathBuf::from).collect::<Vec<_>>();
        convert(&destination, &roots, !only, true)?;
    }
    Ok(())
}
