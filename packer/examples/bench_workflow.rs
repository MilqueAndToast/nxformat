//! Times the everyday commands (scan, pack, recipe, add) on large synthetic
//! DNSP files whose RomFS and DLC data are pseudo-random, so they do not
//! compress. Usage: bench_workflow <work folder> [base MiB]
use nxformat_packer::cli;
use nxformat_packer::native::synthetic::{dnsp, title_entries, TitleSpec};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

const APP: u64 = 0x0100_0000_0001_0000;
const MIB: usize = 1 << 20;

/// Replace every long run of `seed` with pseudo-random bytes. Only the
/// RomFS file and DLC data are that long; their hashes are not checked by the
/// inventory or the conversion.
fn scramble(bytes: &mut [u8], seed: u8) {
    let mut state = 0x9E37_79B9_7F4A_7C15u64 ^ seed as u64;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != seed {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i] == seed {
            i += 1;
        }
        if i - start < 1 << 16 {
            continue;
        }
        for chunk in bytes[start..i].chunks_mut(8) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let v = state.to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
    }
}

fn write(path: &Path, mut spec: TitleSpec, size: usize) {
    if let Some(p) = &mut spec.program {
        p.romfs_bytes = size;
    }
    if spec.data.is_some() {
        spec.data = Some(vec![spec.seed; size]);
    }
    let mut bytes = dnsp(&title_entries(&spec));
    scramble(&mut bytes, spec.seed);
    fs::write(path, bytes).unwrap();
}

fn run(label: &str, args: &[String]) {
    let sink: cli::Sink = Arc::new(|error, line: &str| {
        if error || line.starts_with("PASS") || line.starts_with("@NOTE") {
            println!("    {line}");
        }
    });
    let t = Instant::now();
    cli::run(args.to_vec(), Some(sink), None).unwrap();
    println!("{label:<28} {:>8.2} s", t.elapsed().as_secs_f64());
}

fn s(p: &Path) -> String {
    p.display().to_string()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let work = Path::new(&args[1]);
    let base_mib: usize = args.get(2).map(|a| a.parse().unwrap()).unwrap_or(2048);
    fs::create_dir_all(work).unwrap();
    let base = work.join("base.nsp");
    let update = work.join("update.nsp");
    let update2 = work.join("update2.nsp");
    let dlc = work.join("dlc.nsp");
    if !base.exists() {
        write(&base, TitleSpec::application(APP, 0, 1), base_mib * MIB);
        write(&update, TitleSpec::patch(APP, 65536, 2), base_mib / 4 * MIB);
        write(
            &update2,
            TitleSpec::patch(APP, 131072, 3),
            base_mib / 4 * MIB,
        );
        write(
            &dlc,
            TitleSpec::add_on(APP, APP + 0x1001, 0, 4),
            base_mib / 8 * MIB,
        );
    }
    for f in [&base, &update, &update2, &dlc] {
        println!("{} {} MiB", s(f), fs::metadata(f).unwrap().len() >> 20);
    }
    let cmd = |v: &[&str]| v.iter().map(|a| a.to_string()).collect::<Vec<_>>();
    let nx = work.join("game.nx");
    let nx2 = work.join("game-recipe.nx");
    for p in [&nx, &nx2] {
        let _ = fs::remove_file(p);
    }
    if args.get(3).map(String::as_str) == Some("add") {
        run("pack base", &cmd(&["pack", &s(&base), &s(&nx)]));
        for f in [&update, &update2, &dlc] {
            run("add one", &cmd(&["add", &s(&nx), &s(f)]));
        }
        return;
    }
    run(
        "scan (4 files)",
        &cmd(&["scan", &s(&base), &s(&update), &s(&update2), &s(&dlc)]),
    );
    run("pack base", &cmd(&["pack", &s(&base), &s(&nx)]));
    run("recipe base", &cmd(&["recipe", &s(&nx), &s(&base)]));
    run(
        "pack --recipe base",
        &cmd(&["pack", "--recipe", &s(&base), &s(&nx2)]),
    );
    run(
        "add 3 (update x2, dlc)",
        &cmd(&["add", &s(&nx), &s(&update), &s(&update2), &s(&dlc)]),
    );
    run(
        "add --recipe 3",
        &cmd(&[
            "add",
            "--recipe",
            &s(&nx2),
            &s(&update),
            &s(&update2),
            &s(&dlc),
        ]),
    );
    for p in [&nx, &nx2] {
        println!("{} {} MiB", s(p), fs::metadata(p).unwrap().len() >> 20);
    }
}
