//! Reproducible 64 MiB synthetic streaming benchmark for the RAW writer.

use nxformat_packer::{
    write_archive, ApplicationInput, ArchiveInput, DirectoryInput, EntryInput, FileInput, Platform,
    ProgramInput, ProgramKind, SnapshotInput, SnapshotType, WriterOptions,
};
use std::io::{self, Read};
use std::path::PathBuf;

const INPUT_SIZE: u64 = 64 * 1024 * 1024;

/// Deterministic non-compressed byte pattern generated incrementally.
struct SyntheticReader {
    remaining: u64,
    state: u32,
    word: [u8; 4],
    word_pos: usize,
}
impl SyntheticReader {
    fn next_word(&mut self) {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        self.word = x.to_le_bytes();
        self.word_pos = 0;
    }
}
impl Read for SyntheticReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = (self.remaining.min(output.len() as u64)) as usize;
        for byte in &mut output[..count] {
            if self.word_pos == self.word.len() {
                self.next_word();
            }
            *byte = self.word[self.word_pos];
            self.word_pos += 1;
        }
        self.remaining -= count as u64;
        Ok(count)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("expected destination .nx path")?,
    );
    let program_id = 0x0100_0000_0000_7001;
    let input = ArchiveInput::new(vec![ApplicationInput {
        application_id: 0x0100_0000_0000_7000,
        platform: Platform::Unknown,
        programs: vec![ProgramInput {
            program_id,
            kind: ProgramKind::Unknown,
            content_id: None,
            npdm: None,
        }],
        snapshots: vec![SnapshotInput {
            addon_title_id: None,
            native: Default::default(),
            program_id: Some(program_id),
            snapshot_type: SnapshotType::Base,
            numeric_version: 1,
            content_id: None,
            build_id: None,
            display_version: None,
            dependencies: Vec::new(),
            metadata: None,
            root: DirectoryInput::new(vec![EntryInput::file(
                "synthetic-64m.bin",
                FileInput::new(
                    INPUT_SIZE,
                    SyntheticReader {
                        remaining: INPUT_SIZE,
                        state: 0x6d2b_79f5,
                        word: [0; 4],
                        word_pos: 4,
                    },
                ),
            )]),
        }],
    }]);
    let stats = write_archive(input, &output, &WriterOptions::default())?;
    println!(
        "input_bytes,archive_bytes,file_entries,file_records,blocks,elapsed_ms,input_bytes_per_second,accounted_memory_bytes,peak_temporary_disk_bytes"
    );
    println!(
        "{},{},{},{},{},{},{},{},{}",
        stats.input_bytes,
        stats.archive_bytes,
        stats.file_entries,
        stats.file_records,
        stats.block_count,
        stats.elapsed.as_millis(),
        stats.input_bytes_per_second,
        stats.accounted_memory_bytes,
        stats.peak_temporary_disk_bytes
    );
    Ok(())
}
