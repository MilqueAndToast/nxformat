//! Standalone slice-integrity microbenchmark. Build with rustc -O, no crates.
#[path = "../packer/src/hash.rs"]
#[allow(dead_code)]
mod hash;

use std::hint::black_box;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mib: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "2048".into())
        .parse()?;
    if !(1..=65536).contains(&mib) {
        return Err("expected 1..65536 MiB per timed sample".into());
    }
    let bytes: Vec<u8> = (0..1024 * 1024u32)
        .map(|i| (i.wrapping_mul(17) ^ (i >> 11)) as u8)
        .collect();
    println!("block_kib,method,repeat,bytes,elapsed_ns,gib_per_second");
    for kib in [16, 64, 128, 256, 512, 1024] {
        let input = &bytes[..kib * 1024];
        let expected: Vec<_> = input.chunks(hash::CHECK_SIZE).map(hash::crc32c).collect();
        let mut checks = Vec::with_capacity(expected.len());
        hash::slice_checks(input, &mut checks);
        assert_eq!(checks, expected);
        let iterations = mib * 1024 / kib;
        for repeat in 0..5 {
            // Alternate order to reduce systematic warm-up/frequency bias.
            for method in if repeat % 2 == 0 {
                ["serial", "interleaved"]
            } else {
                ["interleaved", "serial"]
            } {
                let start = Instant::now();
                for _ in 0..iterations {
                    checks.clear();
                    let input = black_box(input);
                    if method == "serial" {
                        checks.extend(input.chunks(hash::CHECK_SIZE).map(hash::crc32c));
                    } else {
                        hash::slice_checks(input, black_box(&mut checks));
                    }
                    black_box(&checks);
                }
                let elapsed = start.elapsed();
                let size = input.len() * iterations;
                println!(
                    "{kib},{method},{repeat},{size},{},{:.3}",
                    elapsed.as_nanos(),
                    size as f64 / 1073741824.0 / elapsed.as_secs_f64()
                );
            }
        }
    }
    Ok(())
}
