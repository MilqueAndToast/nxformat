//! Mount-cost fixture at game scale: `mount_scale_case OUT.nx FILES`.
//! A base and one update with FILES small files each, in directories of 100,
//! the update changing 1 % of them (Mario Kart World has ~200 000 files).
use nxformat_packer::*;
use std::path::PathBuf;
fn tree(files: u32, version: u32) -> DirectoryInput {
    let dirs = (0..files.div_ceil(100))
        .map(|d| {
            let entries = (d * 100..(d * 100 + 100).min(files))
                .map(|i| {
                    let changed = version != 0 && i % 100 == 7;
                    let tag = if changed { i ^ 0x8000_0000 } else { i };
                    let bytes: Vec<u8> = (0..96u32)
                        .map(|k| (tag.wrapping_mul(2654435761) >> (k % 24)) as u8)
                        .collect();
                    EntryInput::file(format!("file_{i:06}.bin"), FileInput::from_bytes(bytes))
                })
                .collect();
            EntryInput::directory(format!("dir_{d:05}"), DirectoryInput::new(entries))
        })
        .collect();
    DirectoryInput::new(vec![EntryInput::directory(
        "romfs",
        DirectoryInput::new(dirs),
    )])
}
/// FILES = 0: names that sort before '/' ("a-b", "a.c", "a b", "a!") beside
/// directories "a" and "a-b", so full-path order differs from the walk order.
fn tricky(version: u32) -> DirectoryInput {
    let file = |n: &str| {
        EntryInput::file(
            n,
            FileInput::from_bytes(format!("{n}{version}").into_bytes()),
        )
    };
    let dir =
        |n: &str, inner: Vec<EntryInput>| EntryInput::directory(n, DirectoryInput::new(inner));
    let mut names: Vec<EntryInput> = ["a b", "a!", "a.c", "a0", "a~", "ab", "\u{e9}"]
        .iter()
        .map(|n| file(n))
        .collect();
    names.push(dir(
        "a",
        vec![
            file("x-y"),
            dir("x.d", vec![file("z")]),
            dir("x", vec![file("q"), dir("e", vec![])]),
        ],
    ));
    names.push(dir("a-b", vec![file("!"), dir(" ", vec![file("deep")])]));
    DirectoryInput::new(names)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).ok_or("output archive")?);
    let files: u32 = std::env::args().nth(2).ok_or("file count")?.parse()?;
    let tree = |version| {
        if files == 0 {
            tricky(version)
        } else {
            tree(files, version)
        }
    };
    let snapshots = (0..2)
        .map(|version| SnapshotInput {
            addon_title_id: None,
            native: Default::default(),
            program_id: Some(1),
            snapshot_type: if version == 0 {
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
            root: tree(version),
        })
        .collect();
    let stats = write_archive(
        ArchiveInput::new(vec![ApplicationInput {
            application_id: 1,
            platform: Platform::Switch1,
            programs: vec![ProgramInput {
                program_id: 1,
                kind: ProgramKind::Primary,
                content_id: None,
                npdm: None,
            }],
            snapshots,
        }]),
        &out,
        &WriterOptions {
            max_memory_bytes: 4 << 30,
            ..Default::default()
        },
    )?;
    println!("archive {} bytes", stats.archive_bytes);
    Ok(())
}
