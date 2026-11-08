//! Synthetic native-inventory benchmark: time, bytes read, heap allocations
//! and peak heap for Fast and Strict modes.
//!
//! ```text
//! cargo run --release --example native_inventory_bench -- [apps] [dlc_per_app] [iterations] [out.csv]
//! ```
//!
//! Optionally `--real PATH` measures a local dump directory instead; real-data
//! rows are labelled `real` and must not be committed with title names.

use nxformat_packer::native::synthetic::{dnsp, title_entries, TitleSpec};
use nxformat_packer::native::{inventory, InventoryMode, InventoryOptions};
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

struct Counting;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static CURRENT: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            let now =
                CURRENT.fetch_add(layout.size() as u64, Ordering::Relaxed) + layout.size() as u64;
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        CURRENT.fetch_sub(layout.size() as u64, Ordering::Relaxed);
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut real = None;
    let mut positional = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--real" {
            real = args.get(i + 1).map(PathBuf::from);
            i += 2;
        } else {
            positional.push(args[i].clone());
            i += 1;
        }
    }
    let apps: u64 = positional
        .first()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    let dlc_per_app: u64 = positional.get(1).and_then(|v| v.parse().ok()).unwrap_or(4);
    let iterations: usize = positional.get(2).and_then(|v| v.parse().ok()).unwrap_or(5);
    let out = positional.get(3).map(PathBuf::from);

    let (root, label, _guard) = match real {
        Some(path) => (path, "real".to_string(), None),
        None => {
            let dir = std::env::temp_dir().join(format!("nx-native-bench-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let started = Instant::now();
            for a in 0..apps {
                let app = 0x0100_0000_0010_0000u64 + (a << 13);
                let seed = (a % 251) as u8;
                let write = |name: String, spec: TitleSpec| {
                    std::fs::write(dir.join(name), dnsp(&title_entries(&spec))).unwrap()
                };
                write(
                    format!("{a}-base.nsp"),
                    TitleSpec::application(app, 0, seed),
                );
                write(
                    format!("{a}-update.nsp"),
                    TitleSpec::patch(app, 0x10000, seed.wrapping_add(1)),
                );
                for d in 0..dlc_per_app {
                    write(
                        format!("{a}-dlc{d}.nsp"),
                        TitleSpec::add_on(app, app + 0x1001 + d, 0, seed.wrapping_add(2)),
                    );
                }
            }
            eprintln!(
                "generated {} synthetic packages in {:.2}s",
                apps * (2 + dlc_per_app),
                started.elapsed().as_secs_f64()
            );
            let label = format!("synthetic apps={apps} dlc_per_app={dlc_per_app}");
            (dir.clone(), label, Some(Cleanup(dir)))
        }
    };

    let mut rows = vec![
        "dataset,mode,iteration,elapsed_ms,sources,metas,bytes_read,allocations,peak_heap_bytes"
            .to_string(),
    ];
    for mode in [InventoryMode::Fast, InventoryMode::Strict] {
        let options = InventoryOptions {
            mode,
            ..InventoryOptions::default()
        };
        for it in 0..iterations {
            ALLOCS.store(0, Ordering::Relaxed);
            let base = CURRENT.load(Ordering::Relaxed);
            PEAK.store(base, Ordering::Relaxed);
            let started = Instant::now();
            let inv = inventory(std::slice::from_ref(&root), &options).unwrap();
            let plan = inv.plan();
            let elapsed = started.elapsed();
            let metas: usize = inv.sources.iter().map(|s| s.metas.len()).sum();
            let row = format!(
                "{label},{mode:?},{it},{:.3},{},{},{},{},{}",
                elapsed.as_secs_f64() * 1000.0,
                inv.sources.len(),
                metas,
                inv.stats.bytes_read,
                ALLOCS.load(Ordering::Relaxed),
                PEAK.load(Ordering::Relaxed) - base
            );
            println!("{row}");
            rows.push(row);
            drop(plan);
            drop(inv);
        }
    }
    if let Some(out) = out {
        std::fs::write(&out, rows.join("\n") + "\n").unwrap();
        eprintln!("wrote {}", out.display());
    }
}

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
