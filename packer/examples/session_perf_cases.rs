//! Stream deterministic fixtures through the production writer, never a whole game buffer.
use nxformat_packer::*;
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
};
#[path = "../../benchmarks/alloc_metrics.rs"]
mod allocations;
#[global_allocator]
static ALLOCATOR: allocations::Counted = allocations::Counted;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !(6..=10).contains(&args.len()) {
        return Err(
            "directory size_mib profile raw|zstd block_kib level [workers] [memory_mib] [single|multi16]".into(),
        );
    }
    let root = PathBuf::from(&args[0]);
    let size = args[1].parse::<u64>()? * 1024 * 1024;
    let profile = &args[2];
    if !matches!(
        profile.as_str(),
        "compressible"
            | "incompressible"
            | "mixed"
            | "marginal"
            | "sample-trap"
            | "gamelike"
            | "graded"
    ) || ![1, 16, 250, 1024].contains(&(size / 1024 / 1024))
    {
        return Err("unsupported synthetic profile/size".into());
    }
    fs::create_dir_all(&root)?;
    let loose = root.join(format!("{profile}_{}.bin", args[1]));
    if !loose.exists() {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&loose)?;
        let mut buffer = [0u8; 64 * 1024];
        let mut state = 0x4e58_464f_524d_4154u64;
        for offset in (0..size).step_by(buffer.len()) {
            if profile == "gamelike" || profile == "graded" {
                gamelike(&mut buffer, offset, &mut state);
                if profile == "graded" {
                    // A share of random words that varies per 256 KiB block
                    // (0..80 %) spreads block savings across the threshold.
                    let share = (offset / 262_144).wrapping_mul(37) % 81;
                    for word in buffer.chunks_exact_mut(8) {
                        state ^= state << 13;
                        state ^= state >> 7;
                        state ^= state << 17;
                        if state % 100 < share {
                            word.copy_from_slice(&state.to_le_bytes());
                        }
                    }
                }
                file.write_all(&buffer)?;
                continue;
            }
            for (i, word) in buffer.chunks_exact_mut(8).enumerate() {
                state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut value = state;
                value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                value ^= value >> 31;
                let random =
                    profile == "incompressible" || (profile == "mixed" && offset / 65536 % 2 == 0);
                if profile == "sample-trap" {
                    // Sparse fixed probes see random bytes although most of
                    // the block is zeros: a deliberate sampling counterexample.
                    if i % 512 < 8 {
                        word.copy_from_slice(&value.to_le_bytes());
                    } else {
                        word.fill(0);
                    }
                } else if profile == "marginal" {
                    if i >= 8192 - 8 {
                        word.fill(0);
                    } else {
                        word.copy_from_slice(&value.to_le_bytes());
                    }
                } else if random {
                    word.copy_from_slice(&value.to_le_bytes());
                } else if i % 512 == 0 {
                    word.copy_from_slice(&(offset + i as u64 * 8).to_le_bytes());
                } else {
                    word.fill(((offset / 4096 + i as u64 / 512) % 251) as u8);
                }
            }
            file.write_all(&buffer)?;
        }
        file.sync_all()?;
    }
    if fs::metadata(&loose)?.len() != size {
        return Err("existing fixture length mismatch".into());
    }
    let codec = &args[3];
    let chunk: u32 = args[4].parse()?;
    let level: i32 = args[5].parse()?;
    let workers: u8 = args.get(6).map_or(Ok(1), |value| value.parse())?;
    let memory_mib: u64 = args.get(7).map_or(Ok(512), |value| value.parse())?;
    let layout = args.get(8).map_or("single", String::as_str);
    let sizing = args.get(9).map_or("fixed", String::as_str);
    if !matches!(
        sizing,
        "fixed"
            | "adaptive-small"
            | "adaptive-medium"
            | "adaptive-sequential"
            | "adaptive-unknown"
            | "adaptive-large"
            | "adaptive-heterogeneous"
    ) {
        return Err("unknown block sizing experiment".into());
    }
    if !matches!(layout, "single" | "multi16") {
        return Err("unknown layout".into());
    }
    let options = WriterOptions {
        compression_mode: match codec.as_str() {
            "raw" => CompressionMode::RawOnly,
            "zstd" => CompressionMode::ZstandardAuto,
            _ => return Err("raw|zstd".into()),
        },
        chunk_size_bytes: chunk * 1024,
        // The block size under test is explicit: no content-based override.
        compressed_chunk_bytes: 0,
        block_sizing: if sizing == "fixed" {
            BlockSizing::Fixed
        } else {
            BlockSizing::Adaptive
        },
        zstd_compression_level: level,
        min_compression_savings_bp: std::env::var("NX_MIN_GAIN_BP")
            .map_or(Ok(2500), |value| value.parse())?,
        compression_workers: workers,
        max_memory_bytes: memory_mib
            .checked_mul(1024 * 1024)
            .ok_or("memory budget overflow")?,
        ..Default::default()
    };
    let count = if layout == "multi16" { 16 } else { 1 };
    let mut entries = Vec::new();
    for index in 0..count {
        let mut source = fs::File::open(&loose)?;
        let length = size / count;
        source.seek(SeekFrom::Start(index * length))?;
        let name = if count == 1 {
            "payload.bin".to_string()
        } else {
            format!("payload{index:02}.bin")
        };
        let mut access = FileReadProfile::default();
        let calibration = if sizing == "adaptive-heterogeneous" {
            match index % 3 {
                0 => "adaptive-medium",
                1 => "adaptive-sequential",
                _ => "adaptive-unknown",
            }
        } else {
            sizing
        };
        for i in 0..10_000u64 {
            match calibration {
                "adaptive-small" => access.observe((i * 7919 % (length / 4096)) * 4096, 4096),
                "adaptive-medium" => access.observe((i * 7919 % (length / 65536)) * 65536, 65536),
                "adaptive-sequential" => access.observe((i % (length / 65536)) * 65536, 65536),
                "adaptive-large" => access.observe(0, length.min(1024 * 1024) as u32),
                _ => break,
            }
        }
        entries.push(EntryInput::file(
            name,
            FileInput::new(length, source.take(length)).with_read_profile(access),
        ));
        if count > 1 {
            let part = root.join(format!("{profile}_{}.part{index:02}.bin", args[1]));
            if !part.exists() {
                let mut source = fs::File::open(&loose)?;
                source.seek(SeekFrom::Start(index * length))?;
                let mut destination = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(part)?;
                std::io::copy(&mut source.take(length), &mut destination)?;
                destination.sync_all()?;
            }
        }
    }
    let input = ArchiveInput::new(vec![ApplicationInput {
        application_id: 1,
        platform: Platform::Switch1,
        programs: vec![ProgramInput {
            program_id: 1,
            kind: ProgramKind::Primary,
            content_id: None,
            npdm: None,
        }],
        snapshots: vec![SnapshotInput {
            addon_title_id: None,
            native: Default::default(),
            program_id: Some(1),
            snapshot_type: SnapshotType::Base,
            numeric_version: 0,
            content_id: None,
            build_id: None,
            display_version: None,
            dependencies: Vec::new(),
            metadata: None,
            root: DirectoryInput::new(entries),
        }],
    }]);
    let suffix = if count == 1 { "" } else { "_multi16" };
    let output = root.join(format!(
        "{profile}_{}_{codec}_{chunk}k_l{level}{suffix}{}.nx",
        args[1],
        if sizing == "fixed" {
            String::new()
        } else {
            format!("_{sizing}")
        }
    ));
    use std::sync::atomic::Ordering::Relaxed;
    let calls_before = allocations::CALLS.load(Relaxed);
    let bytes_before = allocations::BYTES.load(Relaxed);
    let live_before = allocations::LIVE.load(Relaxed);
    allocations::PEAK.store(live_before, Relaxed);
    let stats = write_archive(input, &output, &options)?;
    let allocation_calls = allocations::CALLS.load(Relaxed) - calls_before;
    let allocation_bytes = allocations::BYTES.load(Relaxed) - bytes_before;
    let allocation_peak = allocations::PEAK.load(Relaxed).saturating_sub(live_before);
    let self_peak = allocations::process_peak_rss().map_or(String::new(), |v| v.to_string());
    println!("profile,size_mib,codec,block_kib,level,archive_bytes,raw_blocks,zstd_blocks,dedup_blocks,elapsed_ns,compression_ns,accounted_memory_bytes,temp_peak_bytes,compression_context_peak_bytes,workers,layout,input_read_ns,input_hash_ns,dedup_ns,staging_write_ns,archive_write_ns,verification_ns,compression_stage_ns,compression_staging_read_ns,compression_staging_read_bytes,compression_queue_peak_bytes,rust_allocation_calls,rust_allocated_bytes,rust_peak_increment_bytes,output_sync_ns,atomic_publication_ns,self_peak_rss_bytes,block_sizing");
    println!(
        "{profile},{},{codec},{chunk},{level},{},{},{},{},{},{},{},{},{},{workers},{layout},{},{},{},{},{},{},{},{},{},{},{allocation_calls},{allocation_bytes},{allocation_peak},{},{},{self_peak},{sizing}",
        args[1],
        stats.archive_bytes,
        stats.raw_block_count,
        stats.zstd_block_count,
        stats.deduplicated_blocks,
        stats.elapsed.as_nanos(),
        stats.compression_time.as_nanos(),
        stats.accounted_memory_bytes,
        stats.peak_temporary_disk_bytes,
        stats.compression_context_peak_bytes,
        stats.input_read_time.as_nanos(), stats.input_hash_time.as_nanos(),
        stats.global_dedup_time.as_nanos(), stats.block_staging_write_time.as_nanos(),
        stats.archive_write_time.as_nanos(), stats.output_verification_time.as_nanos(),
        stats.compression_stage_time.as_nanos(), stats.compression_staging_read_time.as_nanos(),
        stats.compression_staging_read_bytes, stats.compression_queue_peak_bytes,
        stats.output_sync_time.as_nanos(), stats.atomic_publication_time.as_nanos()
    );
    Ok(())
}

/// Game-like bytes: 64 KiB chunks alternate between 32-byte binary records
/// (ids, small floats, flags, noise) and word text. Zstd level 3 keeps about
/// a third to a half of it, and decodes at realistic speed.
fn gamelike(buffer: &mut [u8], offset: u64, state: &mut u64) {
    let mut next = || {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    };
    if offset / 65536 % 2 == 0 {
        for (i, r) in buffer.chunks_exact_mut(32).enumerate() {
            let n = next();
            r.fill(0);
            r[..4].copy_from_slice(&((offset / 32) as u32 + i as u32).to_le_bytes());
            r[4..8].copy_from_slice(&(1.0f32 + (n % 1000) as f32 / 1000.0).to_le_bytes());
            r[8..12].copy_from_slice(&((n >> 20) as u32 & 0xff).to_le_bytes());
            r[12] = (n >> 40) as u8 & 3;
            r[16..24].copy_from_slice(&(n & 0x0000_ffff_0000_00ff).to_le_bytes());
        }
    } else {
        const WORDS: [&str; 17] = [
            "mesh", "texture", "shader", "level", "actor", "physics", "sound", "script",
            "material", "anim", "bone", "event", "quest", "item", "<node>", "</node>", "value=",
        ];
        let mut at = 0;
        while at < buffer.len() {
            let w = WORDS[(next() % WORDS.len() as u64) as usize].as_bytes();
            let n = w.len().min(buffer.len() - at);
            buffer[at..at + n].copy_from_slice(&w[..n]);
            at += n;
            if at < buffer.len() {
                buffer[at] = if next() % 8 == 0 { 0x0a } else { b' ' };
                at += 1;
            }
        }
    }
}
