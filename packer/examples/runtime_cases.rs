//! Redistributable access traces for the persistent emulator reader.
use nxformat_packer::*;
use std::{fs, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(std::env::args().nth(1).ok_or("output directory")?);
    fs::create_dir_all(&root)?;
    let data: Vec<u8> = (1..=16).flat_map(|n| vec![n; 256 * 1024]).collect();
    for (name, mode) in [
        ("raw", CompressionMode::RawOnly),
        ("zstd", CompressionMode::ZstandardAuto),
    ] {
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
                root: DirectoryInput::new(vec![
                    EntryInput::file("data.bin", FileInput::from_bytes(data.clone())),
                    EntryInput::file("mirror.bin", FileInput::from_bytes(data.clone())),
                ]),
            })
            .collect();
        write_archive(
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
            root.join(format!("{name}.nx")),
            &WriterOptions {
                compression_mode: mode,
                // Session tests count 256 KiB blocks.
                compressed_chunk_bytes: 0,
                ..Default::default()
            },
        )?;
    }
    Ok(())
}
