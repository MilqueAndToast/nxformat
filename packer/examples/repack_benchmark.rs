use nxformat_packer::{
    inspect_archive, repack_archive, write_archive, ApplicationInput, ArchiveInput,
    DependencyInput, DependencyKind, DirectoryInput, EntryInput, FileInput, IntegrityPolicy,
    Platform, ProgramInput, ProgramKind, RepackPlan, SnapshotInput, SnapshotType, WriterErrorKind,
    WriterOptions, WriterStats,
};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const APP: u64 = 0x0100_0000_0000_3234;
const PROGRAM: u64 = 0x0100_0000_0000_3235;

struct FillReader {
    remaining: u64,
    byte: u8,
}
impl Read for FillReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = self.remaining.min(output.len() as u64) as usize;
        output[..count].fill(self.byte);
        self.remaining -= count as u64;
        Ok(count)
    }
}

struct NoiseReader {
    remaining: u64,
    state: u32,
    word: [u8; 4],
    position: usize,
}
impl Read for NoiseReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = self.remaining.min(output.len() as u64) as usize;
        for byte in &mut output[..count] {
            if self.position == 4 {
                self.state ^= self.state << 13;
                self.state ^= self.state >> 17;
                self.state ^= self.state << 5;
                self.word = self.state.to_le_bytes();
                self.position = 0;
            }
            *byte = self.word[self.position];
            self.position += 1;
        }
        self.remaining -= count as u64;
        Ok(count)
    }
}

fn fill(size: u64, byte: u8) -> FileInput {
    FileInput::new(
        size,
        FillReader {
            remaining: size,
            byte,
        },
    )
}
fn noise(size: u64, seed: u32) -> FileInput {
    FileInput::new(
        size,
        NoiseReader {
            remaining: size,
            state: seed,
            word: [0; 4],
            position: 4,
        },
    )
}

fn base_snapshot(root: DirectoryInput) -> SnapshotInput {
    SnapshotInput {
        addon_title_id: None,
        native: Default::default(),
        program_id: Some(PROGRAM),
        snapshot_type: SnapshotType::Base,
        numeric_version: 0,
        content_id: None,
        build_id: Some([0x41; 32]),
        display_version: None,
        dependencies: vec![],
        metadata: None,
        root,
    }
}

fn update_snapshot(version: u32, base_identity: [u8; 32], root: DirectoryInput) -> SnapshotInput {
    SnapshotInput {
        addon_title_id: None,
        native: Default::default(),
        program_id: Some(PROGRAM),
        snapshot_type: SnapshotType::Update,
        numeric_version: version,
        content_id: None,
        build_id: Some([0x42 + version as u8; 32]),
        display_version: None,
        dependencies: vec![DependencyInput {
            kind: DependencyKind::RequiredBase,
            content_id: None,
            target_snapshot_identity: base_identity,
        }],
        metadata: None,
        root,
    }
}

fn dlc_snapshot(root: DirectoryInput) -> SnapshotInput {
    SnapshotInput {
        addon_title_id: None,
        native: Default::default(),
        program_id: None,
        snapshot_type: SnapshotType::Dlc,
        numeric_version: 1,
        content_id: Some([0x71; 16]),
        build_id: None,
        display_version: None,
        dependencies: vec![],
        metadata: None,
        root,
    }
}

fn app(snapshots: Vec<SnapshotInput>) -> ApplicationInput {
    ApplicationInput {
        application_id: APP,
        platform: Platform::Switch1,
        programs: vec![ProgramInput {
            program_id: PROGRAM,
            kind: ProgramKind::Primary,
            content_id: None,
            npdm: None,
        }],
        snapshots,
    }
}

fn one_file(name: &str, file: FileInput) -> DirectoryInput {
    DirectoryInput::new(vec![EntryInput::file(name, file)])
}

fn many_tiny_files(count: usize) -> DirectoryInput {
    DirectoryInput::new(
        (0..count)
            .map(|index| {
                EntryInput::file(
                    format!("f{index:05}.bin"),
                    fill(512, (index.wrapping_mul(31) & 0xff) as u8),
                )
            })
            .collect(),
    )
}

fn base_input(size: u64, profile: &str) -> ArchiveInput {
    let file = match profile {
        "compressible" => fill(size, 0x26),
        "tiny" => fill(size, 0x17),
        _ => noise(size, 0x1357_9bdf),
    };
    ArchiveInput::new(vec![app(vec![base_snapshot(one_file("base.bin", file))])])
}

fn add_snapshot_plan(snapshot: SnapshotInput, remove_snapshots: Vec<[u8; 32]>) -> RepackPlan {
    let additions = if snapshot.program_id.is_some() {
        ArchiveInput::new(vec![app(vec![snapshot])])
    } else {
        ArchiveInput::new(vec![ApplicationInput {
            application_id: APP,
            platform: Platform::Switch1,
            programs: vec![],
            snapshots: vec![snapshot],
        }])
    };
    RepackPlan {
        remove_snapshots,
        additions,
        metadata_sources: vec![],
    }
}

fn write_archive_checked(input: ArchiveInput, path: &Path, options: &WriterOptions) -> WriterStats {
    write_archive(input, path, options).expect("benchmark setup archive write failed")
}

fn get_id(path: &Path, kind: SnapshotType, version: u32) -> [u8; 32] {
    inspect_archive(path, &WriterOptions::default(), IntegrityPolicy::Fast)
        .expect("benchmark setup archive inspect failed")
        .applications
        .into_iter()
        .flat_map(|app| app.snapshots)
        .find(|snapshot| snapshot.snapshot_type == kind && snapshot.numeric_version == version)
        .expect("benchmark snapshot identity missing")
        .identity
}

fn setup_with_update(
    source: &Path,
    chunk: u32,
    base_size: u64,
    base_profile: &str,
    update_size: u64,
    update_profile: &str,
) -> ([u8; 32], [u8; 32]) {
    let options = WriterOptions {
        chunk_size_bytes: chunk,
        ..WriterOptions::default()
    };
    let base = base_input(base_size, base_profile);
    write_archive_checked(base, source, &options);
    let base_id = get_id(source, SnapshotType::Base, 0);
    let update_file = match update_profile {
        "compressible" => fill(update_size, 0x6a),
        "tiny" => fill(update_size, 0x33),
        _ => noise(update_size, 0x2468_ace1),
    };
    let complete = ArchiveInput::new(vec![app(vec![
        base_snapshot(one_file(
            "base.bin",
            match base_profile {
                "compressible" => fill(base_size, 0x26),
                "tiny" => fill(base_size, 0x17),
                _ => noise(base_size, 0x1357_9bdf),
            },
        )),
        update_snapshot(1, base_id, one_file("update.bin", update_file)),
    ])]);
    write_archive_checked(complete, source, &options);
    (base_id, get_id(source, SnapshotType::Update, 1))
}

fn setup_base(source: &Path, chunk: u32, size: u64, profile: &str) -> [u8; 32] {
    let options = WriterOptions {
        chunk_size_bytes: chunk,
        ..WriterOptions::default()
    };
    write_archive_checked(base_input(size, profile), source, &options);
    get_id(source, SnapshotType::Base, 0)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let scenario = arguments
        .next()
        .ok_or("usage: repack_benchmark <scenario> <directory>")?
        .to_string_lossy()
        .to_string();
    let directory = PathBuf::from(arguments.next().ok_or("missing benchmark directory")?);
    std::fs::create_dir_all(&directory)?;
    let source = directory.join("source.nx");
    let output = directory.join("output.nx");
    let scenario_chunk = scenario
        .strip_prefix("chunk_")
        .and_then(|value| value.parse::<u32>().ok())
        .map(|kib| kib * 1024);
    let options = WriterOptions {
        chunk_size_bytes: scenario_chunk.unwrap_or_else(|| {
            if matches!(scenario.as_str(), "many_tiny_files" | "mixed_chunk_sizes") {
                1024 * 1024
            } else {
                WriterOptions::default().chunk_size_bytes
            }
        }),
        ..WriterOptions::default()
    };

    let plan = match scenario.as_str() {
        "add_small_update" => {
            let base_id = setup_base(&source, 256 * 1024, 64 * 1024 * 1024, "noise");
            let update = update_snapshot(
                1,
                base_id,
                one_file("small-update.bin", fill(1024 * 1024, 0x5c)),
            );
            Some(add_snapshot_plan(update, vec![]))
        }
        "remove_old_update" => {
            let (_, old_id) = setup_with_update(
                &source,
                256 * 1024,
                32 * 1024 * 1024,
                "noise",
                8 * 1024 * 1024,
                "compressible",
            );
            Some(RepackPlan {
                remove_snapshots: vec![old_id],
                additions: ArchiveInput::new(vec![]),
                metadata_sources: vec![],
            })
        }
        "add_dlc" => {
            setup_base(&source, 256 * 1024, 32 * 1024 * 1024, "noise");
            let dlc = dlc_snapshot(one_file("dlc.bin", noise(4 * 1024 * 1024, 0x9876_5432)));
            Some(add_snapshot_plan(dlc, vec![]))
        }
        "replace_update" => {
            let (base_id, old_id) = setup_with_update(
                &source,
                256 * 1024,
                32 * 1024 * 1024,
                "noise",
                8 * 1024 * 1024,
                "compressible",
            );
            let new = update_snapshot(
                2,
                base_id,
                one_file("replacement.bin", fill(8 * 1024 * 1024, 0x7a)),
            );
            Some(add_snapshot_plan(new, vec![old_id]))
        }
        "retain_multiple_updates" => {
            let (base_id, _) = setup_with_update(
                &source,
                256 * 1024,
                16 * 1024 * 1024,
                "noise",
                4 * 1024 * 1024,
                "compressible",
            );
            let newer =
                update_snapshot(2, base_id, one_file("v2.bin", fill(4 * 1024 * 1024, 0x38)));
            Some(add_snapshot_plan(newer, vec![]))
        }
        "delete_only_snapshot" => {
            let opts = WriterOptions::default();
            write_archive_checked(base_input(1024 * 1024, "noise"), &source, &opts);
            let base_id = get_id(&source, SnapshotType::Base, 0);
            Some(RepackPlan {
                remove_snapshots: vec![base_id],
                additions: ArchiveInput::new(vec![]),
                metadata_sources: vec![],
            })
        }
        "cancel_latency" => {
            setup_base(&source, 256 * 1024, 32 * 1024 * 1024, "noise");
            Some(RepackPlan {
                remove_snapshots: vec![],
                additions: ArchiveInput::new(vec![]),
                metadata_sources: vec![],
            })
        }
        "many_tiny_files" => {
            let source_options = WriterOptions {
                chunk_size_bytes: 64 * 1024,
                ..WriterOptions::default()
            };
            let base = ArchiveInput::new(vec![app(vec![base_snapshot(many_tiny_files(4096))])]);
            write_archive_checked(base, &source, &source_options);
            let base_id = get_id(&source, SnapshotType::Base, 0);
            let update = update_snapshot(1, base_id, many_tiny_files(4096));
            Some(add_snapshot_plan(update, vec![]))
        }
        "compressible" | "incompressible" => {
            let base_id = setup_base(&source, 256 * 1024, 4 * 1024 * 1024, "noise");
            let file = if scenario == "compressible" {
                fill(16 * 1024 * 1024, 0x51)
            } else {
                noise(16 * 1024 * 1024, 0x9876_5432)
            };
            let update = update_snapshot(1, base_id, one_file("profile.bin", file));
            Some(add_snapshot_plan(update, vec![]))
        }
        "mixed_chunk_sizes" => {
            let base_id = setup_base(&source, 64 * 1024, 8 * 1024 * 1024, "noise");
            let update = update_snapshot(
                1,
                base_id,
                one_file("one-meg-chunk.bin", noise(4 * 1024 * 1024, 0x7654_3210)),
            );
            Some(add_snapshot_plan(update, vec![]))
        }
        value if value.starts_with("chunk_") => {
            let chunk = scenario_chunk.ok_or(
                "chunk scenario must use chunk_64, chunk_128, chunk_256, chunk_512, or chunk_1024",
            )?;
            let base_id = setup_base(&source, chunk, 16 * 1024 * 1024, "noise");
            let update = update_snapshot(
                1,
                base_id,
                one_file("chunk-profile.bin", noise(4 * 1024 * 1024, 0x4567_89ab)),
            );
            Some(add_snapshot_plan(update, vec![]))
        }
        "full_rebuild" => {
            let (base_id, _) = setup_with_update(
                &source,
                256 * 1024,
                64 * 1024 * 1024,
                "noise",
                1024 * 1024,
                "compressible",
            );
            let archive = ArchiveInput::new(vec![app(vec![
                base_snapshot(one_file("base.bin", noise(64 * 1024 * 1024, 0x1357_9bdf))),
                update_snapshot(
                    1,
                    base_id,
                    one_file("small-update.bin", fill(1024 * 1024, 0x5c)),
                ),
            ])]);
            std::io::stdout().write_all(b"READY\n")?;
            std::io::stdout().flush()?;
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            let start = Instant::now();
            match write_archive(archive, &output, &options) {
                Ok(stats) => print_writer_result(
                    &scenario,
                    "full_rebuild",
                    start.elapsed().as_nanos() as u64,
                    &stats,
                ),
                Err(error) => print_error_result(
                    &scenario,
                    "full_rebuild",
                    start.elapsed().as_nanos() as u64,
                    if error.kind() == WriterErrorKind::Cancelled {
                        "cancelled"
                    } else {
                        "rejected"
                    },
                    &error.to_string(),
                ),
            }
            return Ok(());
        }
        _ => return Err(format!("unknown benchmark scenario: {scenario}").into()),
    };
    let operation = "repack";

    std::io::stdout().write_all(b"READY\n")?;
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let cancellation = if scenario == "cancel_latency" {
        let token = options.cancellation.clone();
        let cancelled_at = Arc::new(Mutex::new(None));
        let thread_timestamp = Arc::clone(&cancelled_at);
        let thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            *thread_timestamp.lock().unwrap() = Some(Instant::now());
            token.cancel();
        });
        Some((thread, cancelled_at))
    } else {
        None
    };
    let start = Instant::now();
    let integrity = if scenario == "cancel_latency" {
        IntegrityPolicy::Strict
    } else {
        IntegrityPolicy::Fast
    };
    let result = repack_archive(
        &source,
        &output,
        plan.expect("repack plan configured"),
        &options,
        integrity,
    );
    let elapsed = start.elapsed().as_nanos() as u64;
    let cancellation_response_ns = if let Some((cancellation_thread, cancelled_at)) = cancellation {
        let _ = cancellation_thread.join();
        cancelled_at
            .lock()
            .unwrap()
            .map(|at| Instant::now().saturating_duration_since(at).as_nanos() as u64)
    } else {
        None
    };
    if let Some(response) = cancellation_response_ns {
        println!("CANCEL,{scenario},{response}");
    }
    match result {
        Ok(stats) => print_repack_result(&scenario, operation, elapsed, &stats),
        Err(error) => print_error_result(
            &scenario,
            operation,
            elapsed,
            if error.kind() == WriterErrorKind::Cancelled {
                "cancelled"
            } else {
                "rejected"
            },
            &error.to_string(),
        ),
    }
    Ok(())
}

fn print_repack_result(
    scenario: &str,
    operation: &str,
    elapsed_ns: u64,
    stats: &nxformat_packer::RepackStats,
) {
    println!(
        "RESULT,{scenario},{operation},ok,{elapsed_ns},{},{},{},{},{},{},{},{},{},{}",
        stats.archive_bytes,
        stats.retained_stored_bytes_copied,
        stats.added_stored_bytes_copied,
        stats.added_blocks_reused,
        stats.added_logical_bytes_considered_for_compression,
        stats.added_logical_bytes_compressed,
        stats.compression_time.as_nanos(),
        stats.source_payload_bytes_verified,
        stats.output_bytes_written,
        stats.peak_temporary_disk_bytes
    );
}

fn print_writer_result(scenario: &str, operation: &str, elapsed_ns: u64, stats: &WriterStats) {
    println!(
        "RESULT,{scenario},{operation},ok,{elapsed_ns},{},0,{},{},{},,{},0,{},{}",
        stats.archive_bytes,
        stats.staged_stored_block_bytes,
        stats.deduplicated_blocks,
        stats.staged_logical_block_bytes,
        stats.compression_time.as_nanos(),
        stats.archive_bytes + 256,
        stats.peak_temporary_disk_bytes
    );
}

fn print_error_result(
    scenario: &str,
    operation: &str,
    elapsed_ns: u64,
    status: &str,
    message: &str,
) {
    println!("RESULT,{scenario},{operation},{status},{elapsed_ns},0,0,0,0,0,0,0,0,0,0");
    if status == "cancelled" {
        eprintln!("operation cancelled: {message}");
    } else {
        eprintln!("operation rejected: {message}");
    }
}
