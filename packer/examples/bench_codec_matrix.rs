//! Synthetic game-like workload: compares compression workers and Zstandard
//! levels on write time, archive size and per-block decode cost.
//!
//! cargo run --release --example bench_codec_matrix -- <out-dir> [MiB]

use nxformat_packer::{
    write_archive, ApplicationInput, ArchiveInput, DirectoryInput, EntryInput, FileInput, Platform,
    ProgramInput, ProgramKind, SnapshotInput, SnapshotType, WriterOptions,
};
use std::path::PathBuf;
use std::time::Instant;

const CHUNK: usize = 256 * 1024;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Four content classes per 256 KiB chunk, in game-like proportions:
/// 35 % already-compressed (random), 35 % structured binary records,
/// 20 % text-like, 10 % exact duplicates of earlier chunks.
fn dataset(total: usize) -> Vec<Vec<u8>> {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let words: Vec<&[u8]> = [
        "mesh", "texture", "shader", "level", "actor", "physics", "sound", "script", "material",
        "anim", "bone", "event", "quest", "item", "<node>", "</node>", "value=",
    ]
    .iter()
    .map(|w| w.as_bytes())
    .collect();
    let mut files: Vec<Vec<u8>> = Vec::new();
    let mut produced = 0usize;
    let mut file = Vec::new();
    while produced < total {
        let class = rng.next() % 100;
        let mut chunk = vec![0u8; CHUNK];
        if class < 35 {
            for c in chunk.chunks_mut(8) {
                let v = rng.next().to_le_bytes();
                c.copy_from_slice(&v[..c.len()]);
            }
        } else if class < 70 {
            // 32-byte records: ids, small floats, flags and noise.
            for (i, r) in chunk.chunks_mut(32).enumerate() {
                let n = rng.next();
                r[..4].copy_from_slice(&(i as u32).to_le_bytes());
                r[4..8].copy_from_slice(&((1.0f32 + (n % 1000) as f32 / 1000.0).to_le_bytes()));
                r[8..12].copy_from_slice(&((n >> 20) as u32 & 0xff).to_le_bytes());
                r[12] = (n >> 40) as u8 & 3;
                r[16..24].copy_from_slice(&(n & 0x0000_ffff_0000_00ff).to_le_bytes());
            }
        } else if class < 90 {
            let mut at = 0;
            while at < CHUNK {
                let w = words[(rng.next() % words.len() as u64) as usize];
                let n = w.len().min(CHUNK - at);
                chunk[at..at + n].copy_from_slice(&w[..n]);
                at += n;
                if at < CHUNK {
                    chunk[at] = if rng.next() % 8 == 0 { b'\n' } else { b' ' };
                    at += 1;
                }
            }
        } else if !file.is_empty() {
            let blocks = file.len() / CHUNK;
            let pick = (rng.next() % blocks as u64) as usize * CHUNK;
            chunk.copy_from_slice(&file[pick..pick + CHUNK]);
        }
        file.extend_from_slice(&chunk);
        produced += CHUNK;
        if file.len() >= 32 * 1024 * 1024 {
            files.push(std::mem::take(&mut file));
        }
    }
    if !file.is_empty() {
        files.push(file);
    }
    files
}

fn input(files: &[Vec<u8>]) -> ArchiveInput {
    let entries = files
        .iter()
        .enumerate()
        .map(|(i, f)| EntryInput::file(format!("data{i:02}.bin"), FileInput::from_bytes(f.clone())))
        .collect();
    ArchiveInput::new(vec![ApplicationInput {
        application_id: 0x0100_0000_0000_7000,
        platform: Platform::Unknown,
        programs: vec![ProgramInput {
            program_id: 0x0100_0000_0000_7001,
            kind: ProgramKind::Unknown,
            content_id: None,
            npdm: None,
        }],
        snapshots: vec![SnapshotInput {
            addon_title_id: None,
            native: Default::default(),
            program_id: Some(0x0100_0000_0000_7001),
            snapshot_type: SnapshotType::Base,
            numeric_version: 0,
            content_id: None,
            build_id: None,
            display_version: None,
            dependencies: Vec::new(),
            metadata: None,
            root: DirectoryInput::new(entries),
        }],
    }])
}

/// Median decode time of the archive's Zstandard blocks, in microseconds.
fn decode_cost(path: &PathBuf) -> (f64, u32) {
    let blob = std::fs::read(path).unwrap();
    let u64_at = |at: usize| u64::from_le_bytes(blob[at..at + 8].try_into().unwrap());
    let u32_at = |at: usize| u32::from_le_bytes(blob[at..at + 4].try_into().unwrap());
    let (index, count) = (u64_at(88) as usize, u64_at(96) as usize);
    let mut decoder = zstd::bulk::Decompressor::new().unwrap();
    let mut out = vec![0u8; 1024 * 1024];
    let mut samples = Vec::new();
    for i in 0..count {
        let row = index + i * 24;
        let (offset, stored, logical) = (
            u64_at(row) as usize,
            u32_at(row + 8) as usize,
            u32_at(row + 12) as usize,
        );
        if blob[row + 16] != 1 {
            continue;
        }
        let started = Instant::now();
        let n = decoder
            .decompress_to_buffer(&blob[offset..offset + stored], &mut out[..logical])
            .unwrap();
        samples.push(started.elapsed().as_secs_f64() * 1e6);
        assert_eq!(n, logical);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = samples.get(samples.len() / 2).copied().unwrap_or(0.0);
    (median, samples.len() as u32)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = PathBuf::from(&args[1]);
    let mib: usize = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(512);
    std::fs::create_dir_all(&out).unwrap();
    let files = dataset(mib * 1024 * 1024);
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    println!("dataset_mib={mib} threads={threads}");
    println!(
        "workers,level,seconds,MiB_per_s,archive_mib,ratio,raw_blocks,zstd_blocks,decode_median_us"
    );
    let mut runs: Vec<(u8, i32)> = Vec::new();
    for w in [1u8, 2, 4, 8] {
        if (w as usize) <= threads.max(1) {
            runs.push((w, 3));
        }
    }
    let wmax = threads.min(4) as u8;
    for level in [1, 6, 9, 12, 15, 19] {
        runs.push((wmax, level));
    }
    for (workers, level) in runs {
        let path = out.join(format!("w{workers}-l{level}.nx"));
        let options = WriterOptions {
            compression_workers: workers,
            zstd_compression_level: level,
            ..WriterOptions::default()
        };
        let started = Instant::now();
        let stats = write_archive(input(&files), &path, &options).unwrap();
        let seconds = started.elapsed().as_secs_f64();
        let (decode, _) = decode_cost(&path);
        println!(
            "{workers},{level},{seconds:.3},{:.0},{:.1},{:.3},{},{},{decode:.1}",
            stats.input_bytes as f64 / seconds / 1048576.0,
            stats.archive_bytes as f64 / 1048576.0,
            stats.archive_bytes as f64 / stats.input_bytes as f64,
            stats.raw_block_count,
            stats.zstd_block_count
        );
        std::fs::remove_file(&path).ok();
    }
}
