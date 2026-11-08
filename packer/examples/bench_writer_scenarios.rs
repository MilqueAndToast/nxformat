//! Reproducible RAW-writer workload generator used for before/after runs.

use nxformat_packer::{
    write_archive, ApplicationInput, ArchiveInput, CompressionMode, DirectoryInput, EntryInput,
    FileInput, Platform, ProgramInput, ProgramKind, SnapshotInput, SnapshotType, WriterOptions,
};
use std::io::{self, Read};
use std::path::PathBuf;

const CHUNK: u64 = 256 * 1024;
const MIB: u64 = 1024 * 1024;
const SIZE: u64 = 64 * MIB;

struct RandomReader {
    size: u64,
    position: u64,
    seed: u32,
    changed_blocks: Vec<u64>,
    current_block: u64,
    state: u32,
    word: [u8; 4],
    word_pos: usize,
}

impl RandomReader {
    fn new(size: u64, seed: u32, changed_blocks: Vec<u64>) -> Self {
        Self {
            size,
            position: 0,
            seed,
            changed_blocks,
            current_block: u64::MAX,
            state: 0,
            word: [0; 4],
            word_pos: 4,
        }
    }

    fn start_block(&mut self, block: u64) {
        let changed = self.changed_blocks.contains(&block);
        let mut state = self
            .seed
            .wrapping_add((block as u32).wrapping_mul(0x9e37_79b9));
        if changed {
            state ^= 0xa5a5_5a5a;
        }
        self.state = if state == 0 { 1 } else { state };
        self.current_block = block;
        self.word_pos = 4;
    }

    fn next_byte(&mut self) -> u8 {
        if self.word_pos == self.word.len() {
            let mut x = self.state;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.state = x;
            self.word = x.to_le_bytes();
            self.word_pos = 0;
        }
        let result = self.word[self.word_pos];
        self.word_pos += 1;
        result
    }
}

impl Read for RandomReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = (self.size - self.position).min(output.len() as u64) as usize;
        for byte in &mut output[..count] {
            let block = self.position / CHUNK;
            if block != self.current_block {
                self.start_block(block);
            }
            *byte = self.next_byte();
            self.position += 1;
        }
        Ok(count)
    }
}

struct RepeatedReader {
    remaining: u64,
    byte: u8,
}

impl Read for RepeatedReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = self.remaining.min(output.len() as u64) as usize;
        output[..count].fill(self.byte);
        self.remaining -= count as u64;
        Ok(count)
    }
}

fn random_file(name: String, size: u64, seed: u32, changed: &[u64]) -> EntryInput {
    EntryInput::file(
        name,
        FileInput::new(size, RandomReader::new(size, seed, changed.to_vec())),
    )
}

fn snapshot(version: u32, root: DirectoryInput) -> SnapshotInput {
    SnapshotInput {
        addon_title_id: None,
        native: Default::default(),
        program_id: Some(0x0100_0000_0000_7001),
        snapshot_type: if version == 1 {
            SnapshotType::Base
        } else {
            SnapshotType::Update
        },
        numeric_version: version,
        content_id: None,
        build_id: None,
        display_version: None,
        dependencies: Vec::new(),
        metadata: None,
        root,
    }
}

fn app(snapshots: Vec<SnapshotInput>) -> ArchiveInput {
    ArchiveInput::new(vec![ApplicationInput {
        application_id: 0x0100_0000_0000_7000,
        platform: Platform::Unknown,
        programs: vec![ProgramInput {
            program_id: 0x0100_0000_0000_7001,
            kind: ProgramKind::Unknown,
            content_id: None,
            npdm: None,
        }],
        snapshots,
    }])
}

fn workload(name: &str) -> Result<ArchiveInput, String> {
    let input = match name {
        "unique-random" => app(vec![snapshot(
            1,
            DirectoryInput::new(vec![random_file("random.bin".into(), SIZE, 0x1234_5678, &[])]),
        )]),
        "repetitive" => app(vec![snapshot(
            1,
            DirectoryInput::new(vec![EntryInput::file(
                "repeat.bin",
                FileInput::new(
                    SIZE,
                    RepeatedReader {
                        remaining: SIZE,
                        byte: 0x5a,
                    },
                ),
            )]),
        )]),
        "mixed-64m" => app(vec![snapshot(
            1,
            DirectoryInput::new(vec![
                random_file("random.bin".into(), 32 * MIB, 0x2468_ace0, &[]),
                EntryInput::file(
                    "repeat.bin",
                    FileInput::new(
                        32 * MIB,
                        RepeatedReader {
                            remaining: 32 * MIB,
                            byte: 0x5a,
                        },
                    ),
                ),
            ]),
        )]),
        "snapshots-sharing-most" => app(vec![
            snapshot(
                1,
                DirectoryInput::new(vec![
                    random_file("stable-a.bin".into(), 48 * MIB, 0x1020_3040, &[]),
                    random_file("stable-b.bin".into(), 16 * MIB, 0x5060_7080, &[]),
                ]),
            ),
            snapshot(
                2,
                DirectoryInput::new(vec![
                    random_file("renamed-a.bin".into(), 48 * MIB, 0x1020_3040, &[]),
                    random_file("stable-b.bin".into(), 16 * MIB, 0x5060_7080, &[]),
                ]),
            ),
        ]),
        "limited-modifications" => app(vec![
            snapshot(
                1,
                DirectoryInput::new(vec![random_file("rom.bin".into(), SIZE, 0x89ab_cdef, &[])]),
            ),
            snapshot(
                2,
                DirectoryInput::new(vec![random_file(
                    "rom.bin".into(),
                    SIZE,
                    0x89ab_cdef,
                    &[127, 128],
                )]),
            ),
        ]),
        "many-small-files" => {
            let count = 16_384u32;
            let entries = (0..count)
                .map(|index| {
                    random_file(
                        format!("file-{index:05}.bin"),
                        4 * 1024,
                        index.wrapping_add(1),
                        &[],
                    )
                })
                .collect();
            app(vec![snapshot(1, DirectoryInput::new(entries))])
        }
        _ => {
            return Err(format!(
                "unknown workload {name}; expected unique-random, repetitive, mixed-64m, snapshots-sharing-most, limited-modifications, or many-small-files"
            ))
        }
    };
    Ok(input)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let name = args.next().ok_or("expected workload name")?;
    let output = PathBuf::from(args.next().ok_or("expected destination .nx path")?);
    let codec = args.next().unwrap_or_else(|| "raw".to_owned());
    let level = args.next().map(|v| v.parse()).transpose()?.unwrap_or(3);
    let chunk_kib = args
        .next()
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(256u32);
    let verify = args
        .next()
        .map(|v| v.parse::<bool>())
        .transpose()?
        .unwrap_or(true);
    if args.next().is_some() {
        return Err("too many arguments; expected [raw|zstd] [level] [chunk_kib] [verify]".into());
    }
    let input = workload(&name)?;
    let compression_mode = match codec.as_str() {
        "raw" => CompressionMode::RawOnly,
        "zstd" => CompressionMode::ZstandardAuto,
        _ => return Err("codec must be raw or zstd".into()),
    };
    let options = WriterOptions {
        compression_mode,
        zstd_compression_level: level,
        chunk_size_bytes: chunk_kib * 1024,
        verify_output_payloads: verify,
        ..WriterOptions::default()
    };
    let stats = write_archive(input, &output, &options)?;
    println!(
        "scenario,codec,level,chunk_kib,verify,input_bytes,archive_bytes,file_entries,file_records,blocks,raw_blocks,zstd_blocks,deduplicated_blocks,logical_stage_bytes,stored_stage_bytes,block_spool_bytes,elapsed_us,input_read_hash_us,dedup_us,compression_us,archive_write_us,verification_us,input_bytes_per_second,accounted_memory_bytes,peak_temporary_disk_bytes"
    );
    println!(
        "{name},{codec},{level},{chunk_kib},{verify},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
        stats.input_bytes,
        stats.archive_bytes,
        stats.file_entries,
        stats.file_records,
        stats.block_count,
        stats.raw_block_count,
        stats.zstd_block_count,
        stats.deduplicated_blocks,
        stats.staged_logical_block_bytes,
        stats.staged_stored_block_bytes,
        stats.staged_block_bytes,
        stats.elapsed.as_micros(),
        stats.input_read_hash_time.as_micros(),
        stats.global_dedup_time.as_micros(),
        stats.compression_time.as_micros(),
        stats.archive_write_time.as_micros(),
        stats.output_verification_time.as_micros(),
        stats.input_bytes_per_second,
        stats.accounted_memory_bytes,
        stats.peak_temporary_disk_bytes
    );
    Ok(())
}
