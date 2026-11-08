//! Hostile directory graphs (compare NUP issue #27): a cycle, a directory
//! reached twice and a File record reached twice inside one snapshot. Every
//! Rust entry point must reject them, so a passing `verify` never vouches
//! for a tree that would amplify or loop. The same fixtures run through the
//! C++ reader (tests/conformance/test_reader.py) and the independent Python
//! validator.
use nxformat_packer::*;
use std::{fs, path::PathBuf};

const HOSTILE: [&str; 3] = [
    "cyclic_directory.nx",
    "repeated_directory.nx",
    "repeated_file.nx",
];

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures")
        .join(name)
}

#[test]
fn hostile_graphs_are_rejected_by_every_rust_entry_point() {
    let options = WriterOptions::default();
    for name in HOSTILE {
        let path = fixture(name);
        for policy in [IntegrityPolicy::Fast, IntegrityPolicy::Strict] {
            let error = inspect_archive(&path, &options, policy)
                .err()
                .unwrap_or_else(|| panic!("{name}: inspect {policy:?} accepted"));
            assert!(
                error.to_string().contains("cyclic, shared internally")
                    || error.to_string().contains("File record repeated"),
                "{name}: rejected for another reason: {error}"
            );
        }
        // `verify` and `info` run through the CLI entry point.
        for command in ["verify", "info"] {
            let args = vec![command.to_owned(), path.display().to_string()];
            let sink: cli::Sink = std::sync::Arc::new(|_, _| {});
            assert!(
                cli::run(args, Some(sink), None).is_err(),
                "{name}: {command} accepted"
            );
        }
        // Editing entry points parse the same tree before touching the file.
        let dir = std::env::temp_dir().join(format!("nx-hostile-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let copy = dir.join(name);
        fs::copy(&path, &copy).unwrap();
        let before = fs::read(&copy).unwrap();
        assert!(
            compact_archive(&copy, &options, IntegrityPolicy::Fast).is_err(),
            "{name}: compact accepted"
        );
        assert_eq!(
            fs::read(&copy).unwrap(),
            before,
            "{name}: compact changed the file"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn the_same_trees_without_the_repeat_are_accepted() {
    // Guards the fixtures: their base archives are valid, so the rejections
    // above come from the graph, not from a damaged header or hash.
    let options = WriterOptions::default();
    for name in ["nested_vfs.nx", "shared_snapshots.nx", "shared_blocks.nx"] {
        inspect_archive(fixture(name), &options, IntegrityPolicy::Strict)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

/// The whole fixed-byte corpus (tests/fixtures/fixture_index.json), not only
/// writer-to-reader round trips (compare NUP PR #20, where shifted TLV IDs
/// passed round-trip tests): every `accept` fixture opens under Strict, every
/// `reject` fixture is refused under Fast, every `reject_full_verify` one
/// opens under Fast and is refused under Strict.
#[test]
fn rust_reader_matches_the_binary_fixture_corpus() {
    let index = fs::read_to_string(fixture("fixture_index.json")).unwrap();
    let options = WriterOptions::default();
    let mut checked = 0;
    let mut failures = Vec::new();
    let mut file = None;
    for line in index.lines().map(str::trim) {
        let value = |key: &str| {
            line.strip_prefix(&format!("\"{key}\": \""))
                .and_then(|v| v.split('"').next())
                .map(str::to_owned)
        };
        if let Some(name) = value("file") {
            file = Some(name);
        } else if let Some(expected) = value("expected") {
            let name = file.take().expect("expected before file");
            let path = fixture(&name);
            let fast = inspect_archive(&path, &options, IntegrityPolicy::Fast);
            let strict = inspect_archive(&path, &options, IntegrityPolicy::Strict);
            let ok = match expected.as_str() {
                "accept" => strict.is_ok(),
                "reject" => fast.is_err(),
                "reject_full_verify" => fast.is_ok() && strict.is_err(),
                other => panic!("{name}: unknown expectation {other}"),
            };
            if !ok {
                failures.push(format!("{name} (expected {expected})"));
            }
            checked += 1;
        }
    }
    assert!(
        failures.is_empty(),
        "Rust disagrees with the corpus: {failures:?}"
    );
    assert!(checked >= 61, "only {checked} fixtures read from the index");
}
