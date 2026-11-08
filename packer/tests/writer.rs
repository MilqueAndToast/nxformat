use nxformat_packer::{
    write_archive, ApplicationInput, ArchiveInput, CancellationToken, CompressionMode,
    DirectoryInput, EntryInput, FileInput, Platform, ProgramInput, ProgramKind, SnapshotInput,
    SnapshotType, WriterErrorKind, WriterOptions,
};
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static NEXT: AtomicU64 = AtomicU64::new(0);
struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "nx-writer-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn base_input(root: DirectoryInput, version: u32) -> ArchiveInput {
    let program_id = 0x0100_0000_0000_0001;
    ArchiveInput::new(vec![ApplicationInput {
        application_id: 0x0100_0000_0000_0002,
        platform: Platform::Switch1,
        programs: vec![ProgramInput {
            program_id,
            kind: ProgramKind::Primary,
            content_id: None,
            npdm: None,
        }],
        snapshots: vec![SnapshotInput {
            addon_title_id: None,
            native: Default::default(),
            program_id: Some(program_id),
            snapshot_type: SnapshotType::Base,
            numeric_version: version,
            content_id: None,
            build_id: None,
            display_version: None,
            dependencies: Vec::new(),
            metadata: None,
            root,
        }],
    }])
}

fn make_bytes_input() -> ArchiveInput {
    base_input(
        DirectoryInput::new(vec![
            EntryInput::directory(
                "nested",
                DirectoryInput::new(vec![EntryInput::file(
                    "value.bin",
                    FileInput::from_bytes((0..700_123).map(|i| (i % 251) as u8).collect()),
                )]),
            ),
            EntryInput::file("small.txt", FileInput::from_bytes(b"small".to_vec())),
        ]),
        1,
    )
}

#[test]
fn deterministic_raw_archive_and_streamed_multi_block_input() {
    let temp = TestDir::new();
    let a = temp.path().join("a.nx");
    let b = temp.path().join("b.nx");
    let raw = WriterOptions {
        compression_mode: CompressionMode::RawOnly,
        ..WriterOptions::default()
    };
    let sa = write_archive(make_bytes_input(), &a, &raw).unwrap();
    let sb = write_archive(make_bytes_input(), &b, &raw).unwrap();
    assert_eq!(fs::read(a).unwrap(), fs::read(b).unwrap());
    assert_eq!(sa.input_bytes, 700_128);
    assert_eq!(sa.file_entries, 2);
    assert_eq!(sa.file_records, 2);
    assert_eq!(sa.block_count, 4);
    assert_eq!(sa.deduplicated_blocks, 0);
    assert_eq!(sa.staged_block_bytes, 0);
    assert_eq!(sa.staged_logical_block_bytes, 700_123 + 5);
    assert_eq!(sa.staged_stored_block_bytes, 700_123 + 5);
    assert_eq!(sa.raw_block_count, 4);
    assert_eq!(sa.zstd_block_count, 0);
    assert_eq!(sa.archive_bytes, sb.archive_bytes);
    assert!(sa.accounted_memory_bytes < WriterOptions::default().max_memory_bytes);
    assert!(sa.peak_temporary_disk_bytes >= sa.archive_bytes);
}

#[test]
fn dedup_index_limit_rejects_before_staging_and_preserves_destination() {
    let temp = TestDir::new();
    let out = temp.path().join("archive.nx");
    fs::write(&out, b"previous archive").unwrap();
    let options = WriterOptions {
        max_dedup_index_bytes: 0,
        ..WriterOptions::default()
    };
    let input = base_input(
        DirectoryInput::new(vec![EntryInput::file(
            "unique.bin",
            FileInput::from_bytes(vec![0x71; 32 * 1024]),
        )]),
        1,
    );
    let error = write_archive(input, &out, &options).unwrap_err();
    assert_eq!(error.kind(), WriterErrorKind::ResourceLimit);
    assert!(error.message().contains("deduplication index"));
    assert_eq!(fs::read(&out).unwrap(), b"previous archive");
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[test]
fn resource_limits_reject_before_publish() {
    let temp = TestDir::new();
    let out = temp.path().join("archive.nx");
    fs::write(&out, b"previous archive").unwrap();
    let options = WriterOptions {
        max_memory_bytes: 655_360,
        compression_mode: CompressionMode::RawOnly,
        ..WriterOptions::default()
    };
    let error = write_archive(make_bytes_input(), &out, &options).unwrap_err();
    assert_eq!(error.kind(), WriterErrorKind::ResourceLimit);
    assert_eq!(fs::read(out).unwrap(), b"previous archive");
}

#[test]
fn unsafe_and_duplicate_paths_are_rejected() {
    for entries in [
        vec![EntryInput::file(
            "../escape",
            FileInput::from_bytes(vec![1]),
        )],
        vec![
            EntryInput::file("same", FileInput::from_bytes(vec![1])),
            EntryInput::file("same", FileInput::from_bytes(vec![2])),
        ],
    ] {
        let temp = TestDir::new();
        let out = temp.path().join("archive.nx");
        fs::write(&out, b"old").unwrap();
        let error = write_archive(
            base_input(DirectoryInput::new(entries), 1),
            &out,
            &WriterOptions::default(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), WriterErrorKind::InvalidInput);
        assert_eq!(fs::read(out).unwrap(), b"old");
    }
}

struct FailingReader {
    returned: usize,
    fail_at: usize,
}

struct CancellingReader {
    remaining: usize,
    token: CancellationToken,
}
impl Read for CancellingReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let count = self.remaining.min(out.len());
        out[..count].fill(0x37);
        self.remaining -= count;
        if count != 0 {
            self.token.cancel();
        }
        Ok(count)
    }
}
impl Read for FailingReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.returned >= self.fail_at {
            return Err(io::Error::other("synthetic input failure"));
        }
        let n = out.len().min(self.fail_at - self.returned);
        out[..n].fill(0x5a);
        self.returned += n;
        Ok(n)
    }
}

#[test]
fn input_io_failure_preserves_destination_and_cleans_owned_temps() {
    let temp = TestDir::new();
    let out = temp.path().join("archive.nx");
    fs::write(&out, b"previous archive").unwrap();
    let input = base_input(
        DirectoryInput::new(vec![EntryInput::file(
            "failure.bin",
            FileInput::new(
                1024,
                FailingReader {
                    returned: 0,
                    fail_at: 100,
                },
            ),
        )]),
        1,
    );
    let error = write_archive(input, &out, &WriterOptions::default()).unwrap_err();
    assert_eq!(error.kind(), WriterErrorKind::Io);
    assert_eq!(fs::read(&out).unwrap(), b"previous archive");
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[test]
fn publication_failure_preserves_existing_directory() {
    let temp = TestDir::new();
    let out = temp.path().join("destination");
    fs::create_dir(&out).unwrap();
    fs::write(out.join("sentinel"), b"keep").unwrap();
    let error = write_archive(make_bytes_input(), &out, &WriterOptions::default()).unwrap_err();
    assert_eq!(error.kind(), WriterErrorKind::Io);
    assert_eq!(fs::read(out.join("sentinel")).unwrap(), b"keep");
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[test]
fn successful_write_replaces_an_existing_archive_file() {
    let temp = TestDir::new();
    let out = temp.path().join("archive.nx");
    fs::write(&out, b"older archive").unwrap();
    let stats = write_archive(make_bytes_input(), &out, &WriterOptions::default()).unwrap();
    let bytes = fs::read(&out).unwrap();
    assert_eq!(bytes.get(..4), Some(&b"NXAR"[..]));
    assert_eq!(bytes.len() as u64, stats.archive_bytes);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[test]
fn pre_cancelled_write_does_not_replace_destination() {
    let temp = TestDir::new();
    let out = temp.path().join("archive.nx");
    fs::write(&out, b"old bytes").unwrap();
    let options = WriterOptions::default();
    options.cancellation.cancel();
    let error = write_archive(make_bytes_input(), &out, &options).unwrap_err();
    assert_eq!(error.kind(), WriterErrorKind::Cancelled);
    assert_eq!(fs::read(out).unwrap(), b"old bytes");
}

#[test]
fn cancellation_after_staging_begins_preserves_destination_and_cleans_temps() {
    let temp = TestDir::new();
    let out = temp.path().join("archive.nx");
    fs::write(&out, b"previous archive").unwrap();
    let token = CancellationToken::default();
    let input = base_input(
        DirectoryInput::new(vec![EntryInput::file(
            "cancel.bin",
            FileInput::new(
                700_000,
                CancellingReader {
                    remaining: 700_000,
                    token: token.clone(),
                },
            ),
        )]),
        1,
    );
    let options = WriterOptions {
        cancellation: token,
        ..WriterOptions::default()
    };
    let error = write_archive(input, &out, &options).unwrap_err();
    assert_eq!(error.kind(), WriterErrorKind::Cancelled);
    assert_eq!(fs::read(&out).unwrap(), b"previous archive");
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[test]
fn rejects_trailing_stream_bytes() {
    let temp = TestDir::new();
    let out = temp.path().join("archive.nx");
    let extra = base_input(
        DirectoryInput::new(vec![EntryInput::file(
            "x",
            FileInput::new(1, Cursor::new(vec![1, 2])),
        )]),
        1,
    );
    assert_eq!(
        write_archive(extra, &out, &WriterOptions::default())
            .unwrap_err()
            .kind(),
        WriterErrorKind::InvalidInput
    );
}

#[test]
fn rejects_zero_identifiers_and_incorrect_program_associations() {
    let temp = TestDir::new();
    let out = temp.path().join("archive.nx");
    let mut input = base_input(DirectoryInput::empty(), 1);
    input.applications[0].programs[0].content_id = Some([0; 16]);
    assert_eq!(
        write_archive(input, &out, &WriterOptions::default())
            .unwrap_err()
            .kind(),
        WriterErrorKind::InvalidInput
    );

    let mut input = base_input(DirectoryInput::empty(), 1);
    input.applications[0].snapshots[0].program_id = Some(0x0100_0000_0000_00ff);
    assert_eq!(
        write_archive(input, &out, &WriterOptions::default())
            .unwrap_err()
            .kind(),
        WriterErrorKind::InvalidInput
    );

    let mut input = base_input(DirectoryInput::empty(), 1);
    input.applications[0].snapshots[0].build_id = Some([0; 32]);
    assert_eq!(
        write_archive(input, &out, &WriterOptions::default())
            .unwrap_err()
            .kind(),
        WriterErrorKind::InvalidInput
    );
}

struct CountedReader(std::sync::Arc<AtomicU64>);
impl Read for CountedReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.0.fetch_add(out.len() as u64, Ordering::Relaxed);
        out.fill(42);
        Ok(out.len())
    }
}
#[test]
fn impossible_raw_output_rejects_without_reading_source() {
    let temp = TestDir::new();
    let out = temp.path().join("archive.nx");
    fs::write(&out, b"original").unwrap();
    let count = std::sync::Arc::new(AtomicU64::new(0));
    let input = base_input(
        DirectoryInput::new(vec![EntryInput::file(
            "huge",
            FileInput::new(4 * 1024 * 1024, CountedReader(count.clone())),
        )]),
        1,
    );
    let options = WriterOptions {
        max_archive_size: 4096,
        compression_mode: CompressionMode::RawOnly,
        ..Default::default()
    };
    assert_eq!(
        write_archive(input, &out, &options).unwrap_err().kind(),
        WriterErrorKind::ResourceLimit
    );
    assert_eq!(count.load(Ordering::Relaxed), 0);
    assert_eq!(fs::read(&out).unwrap(), b"original");
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[test]
fn zstd_can_fit_logical_input_larger_than_archive_limit() {
    let temp = TestDir::new();
    let out = temp.path().join("compressed.nx");
    let options = WriterOptions {
        max_archive_size: 12 * 1024,
        ..WriterOptions::default()
    };
    let stats = write_archive(
        base_input(
            DirectoryInput::new(vec![EntryInput::file(
                "large-repeat.bin",
                FileInput::from_bytes(vec![0x35; 4 * 1024 * 1024]),
            )]),
            1,
        ),
        &out,
        &options,
    )
    .unwrap();
    assert!(stats.archive_bytes <= options.max_archive_size);
    assert_eq!(stats.block_count, 1);
    assert_eq!(stats.zstd_block_count, 1);
}
#[test]
fn temporary_budget_covers_staging_and_final_output() {
    for limit in [256, 4096, 262144, 600000] {
        let temp = TestDir::new();
        let out = temp.path().join("archive.nx");
        fs::write(&out, b"original").unwrap();
        let options = WriterOptions {
            max_temporary_disk_bytes: limit,
            compression_mode: CompressionMode::RawOnly,
            ..Default::default()
        };
        assert_eq!(
            write_archive(make_bytes_input(), &out, &options)
                .unwrap_err()
                .kind(),
            WriterErrorKind::ResourceLimit
        );
        assert_eq!(fs::read(&out).unwrap(), b"original");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }
    let temp = TestDir::new();
    let baseline = write_archive(
        make_bytes_input(),
        temp.path().join("baseline.nx"),
        &WriterOptions {
            compression_mode: CompressionMode::RawOnly,
            ..WriterOptions::default()
        },
    )
    .unwrap();
    let options = WriterOptions {
        max_temporary_disk_bytes: baseline.peak_temporary_disk_bytes,
        compression_mode: CompressionMode::RawOnly,
        ..Default::default()
    };
    let exact = write_archive(make_bytes_input(), temp.path().join("exact.nx"), &options).unwrap();
    assert_eq!(
        baseline.peak_temporary_disk_bytes,
        exact.peak_temporary_disk_bytes
    );
}
#[test]
fn ambiguous_program_npdm_is_rejected() {
    let temp = TestDir::new();
    let mut input = base_input(DirectoryInput::empty(), 1);
    let mut second = base_input(DirectoryInput::empty(), 2)
        .applications
        .remove(0)
        .snapshots
        .remove(0);
    second.snapshot_type = SnapshotType::Update;
    input.applications[0].snapshots.push(second);
    input.applications[0].programs[0].npdm = Some(nxformat_packer::MediaInput::from_bytes(
        nxformat_packer::MediaType::Npdm,
        vec![1],
    ));
    assert_eq!(
        write_archive(input, temp.path().join("bad.nx"), &WriterOptions::default())
            .unwrap_err()
            .kind(),
        WriterErrorKind::InvalidInput
    );
}

struct CorruptStagingAtEof {
    source: Cursor<Vec<u8>>,
    directory: PathBuf,
    corrupted: bool,
}
impl Read for CorruptStagingAtEof {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = self.source.read(out)?;
        if n == 0 && !self.corrupted {
            use std::io::{Seek, SeekFrom, Write};
            for entry in fs::read_dir(&self.directory)? {
                let entry = entry?;
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".nx-output")
                {
                    let mut file = fs::OpenOptions::new().write(true).open(entry.path())?;
                    // First block byte; earlier blocks are already flushed.
                    file.seek(SeekFrom::Start(8192))?;
                    file.write_all(&[99])?;
                    self.corrupted = true;
                }
            }
        }
        Ok(n)
    }
}
#[test]
fn readback_rejects_corrupted_staging_before_replacing_destination() {
    let temp = TestDir::new();
    let out = temp.path().join("archive.nx");
    fs::write(&out, b"original").unwrap();
    let input = base_input(
        DirectoryInput::new(vec![EntryInput::file(
            "payload",
            FileInput::new(
                3 * 1024 * 1024,
                CorruptStagingAtEof {
                    source: Cursor::new((0..3 * 1024 * 1024).map(|i| (i % 251) as u8).collect()),
                    directory: temp.path().to_path_buf(),
                    corrupted: false,
                },
            ),
        )]),
        1,
    );
    assert_eq!(
        write_archive(
            input,
            &out,
            &WriterOptions {
                compression_mode: CompressionMode::RawOnly,
                ..WriterOptions::default()
            }
        )
        .unwrap_err()
        .kind(),
        WriterErrorKind::Integrity
    );
    assert_eq!(fs::read(&out).unwrap(), b"original");
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
}
#[test]
fn temporary_budget_also_limits_native_media() {
    let temp = TestDir::new();
    let mut input = base_input(DirectoryInput::empty(), 1);
    input.applications[0].snapshots[0].native.npdm = Some(nxformat_packer::MediaInput::from_bytes(
        nxformat_packer::MediaType::Npdm,
        vec![0; 131072],
    ));
    let out = temp.path().join("archive.nx");
    fs::write(&out, b"original").unwrap();
    let options = WriterOptions {
        max_temporary_disk_bytes: 65536,
        ..Default::default()
    };
    assert_eq!(
        write_archive(input, &out, &options).unwrap_err().kind(),
        WriterErrorKind::ResourceLimit
    );
    assert_eq!(fs::read(&out).unwrap(), b"original");
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
}

struct OneByteCancellingReader {
    token: CancellationToken,
    reads: Arc<AtomicU64>,
}
impl Read for OneByteCancellingReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        self.reads.fetch_add(1, Ordering::Relaxed);
        out[0] = 0x37;
        self.token.cancel();
        Ok(1)
    }
}

#[test]
fn cancellation_interrupts_fragmented_reads_within_one_chunk() {
    let temp = TestDir::new();
    let out = temp.path().join("archive.nx");
    fs::write(&out, b"original").unwrap();
    let token = CancellationToken::default();
    let reads = Arc::new(AtomicU64::new(0));
    let input = base_input(
        DirectoryInput::new(vec![EntryInput::file(
            "slow.bin",
            FileInput::new(
                256 * 1024,
                OneByteCancellingReader {
                    token: token.clone(),
                    reads: Arc::clone(&reads),
                },
            ),
        )]),
        1,
    );
    let options = WriterOptions {
        cancellation: token,
        ..WriterOptions::default()
    };
    let error = write_archive(input, &out, &options).unwrap_err();
    assert_eq!(error.kind(), WriterErrorKind::Cancelled);
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    assert_eq!(fs::read(out).unwrap(), b"original");
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn long_valid_destination_basename_keeps_short_temporary_names() {
    let temp = TestDir::new();
    let name = format!("{}.nx", "x".repeat(236));
    let out = temp.path().join(name);
    write_archive(
        base_input(DirectoryInput::empty(), 1),
        &out,
        &WriterOptions::default(),
    )
    .unwrap();
    assert!(out.exists());
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
}
