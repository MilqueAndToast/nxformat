//! Strict full repack of a real archive to a new file; prints time and stats.
use nxformat_packer::{repack_archive, IntegrityPolicy, RepackPlan, WriterOptions};
use std::time::Instant;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let started = Instant::now();
    let stats = repack_archive(
        &args[1],
        &args[2],
        RepackPlan::new(),
        &WriterOptions::default(),
        IntegrityPolicy::Strict,
    )
    .expect("repack");
    println!(
        "REPACK seconds={:.2} decodes={} out_decodes={} bytes={}",
        started.elapsed().as_secs_f64(),
        stats.source_verification.block_decodes,
        stats.output_verification.block_decodes,
        stats.archive_bytes
    );
}
