//! Local, read-only inventory of user-supplied decrypted dumps.
//!
//! ```text
//! cargo run --release --example native_inventory -- [--strict] [--json out.json] PATH...
//! ```
//!
//! Prints a per-application summary and every issue. Nothing is written next
//! to the sources; real-data results must not be committed or uploaded.

use nxformat_packer::native::{inventory, InventoryMode, InventoryOptions, Severity};
use std::path::PathBuf;

fn main() {
    let mut options = InventoryOptions::default();
    let mut json = None;
    let mut roots = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--strict" => options.mode = InventoryMode::Strict,
            "--json" => json = args.next().map(PathBuf::from),
            _ => roots.push(PathBuf::from(a)),
        }
    }
    if roots.is_empty() {
        eprintln!("usage: native_inventory [--strict] [--json out.json] PATH...");
        std::process::exit(2);
    }
    let inv = match inventory(&roots, &options) {
        Ok(inv) => inv,
        Err(e) => {
            eprintln!("inventory failed: {e}");
            std::process::exit(1);
        }
    };
    let plan = inv.plan();
    let mut by_format = std::collections::BTreeMap::new();
    for s in &inv.sources {
        *by_format
            .entry(format!("{:?}/{:?}", s.format, s.status))
            .or_insert(0) += 1;
    }
    println!(
        "mode={:?} sources={} ignored={} bytes_read={} elapsed={:.3}s",
        options.mode,
        inv.stats.files_examined,
        inv.stats.files_ignored,
        inv.stats.bytes_read,
        inv.stats.elapsed.as_secs_f64()
    );
    for (k, v) in &by_format {
        println!("  {k}: {v}");
    }
    println!(
        "applications={} planned_applications={} planned_snapshots={} blocked={}",
        inv.applications.len(),
        plan.applications.len(),
        plan.applications
            .iter()
            .map(|a| a.snapshots.len())
            .sum::<usize>(),
        plan.blocked.len()
    );
    let mut counts = std::collections::BTreeMap::new();
    for i in inv.all_issues() {
        *counts
            .entry(format!("{:?}:{:?}", i.severity, i.code))
            .or_insert(0) += 1;
    }
    for (k, v) in &counts {
        println!("  issue {k} x{v}");
    }
    for i in inv.all_issues().filter(|i| i.severity >= Severity::Warning) {
        let src = i
            .source
            .map(|s| inv.sources[s].path.display().to_string())
            .unwrap_or_default();
        println!("{:?} {:?} {} {}", i.severity, i.code, i.message, src);
    }
    for s in &inv.sources {
        for i in s.issues.iter().filter(|i| i.severity >= Severity::Warning) {
            println!("  @{} :: {:?} {}", s.path.display(), i.code, i.message);
        }
    }
    for b in &plan.blocked {
        println!(
            "blocked {:016x} v{} {:?} ({}): {}",
            b.title_id,
            b.version,
            b.meta_type,
            b.source.display(),
            b.reasons.join("; ")
        );
    }
    if let Some(path) = json {
        std::fs::write(&path, inv.to_json()).expect("write JSON");
        println!("json written to {}", path.display());
    }
}
