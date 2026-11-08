use nxformat_packer::{
    inspect_archive, repack_archive, write_archive, ApplicationInput, ArchiveInput,
    DependencyInput, DependencyKind, DirectoryInput, EntryInput, FileInput, IntegrityPolicy,
    LocalizedMetadata, MediaInput, MediaType, MetadataSourceSelection, Platform, ProgramInput,
    ProgramKind, RepackPlan, SnapshotInput, SnapshotMetadata, SnapshotType, WriterOptions,
};
use std::fs;
use std::path::PathBuf;

const APP: u64 = 0x0100_0000_0000_2234;
const PROGRAM: u64 = 0x0100_0000_0000_2235;

fn input(bytes: Vec<u8>) -> FileInput {
    FileInput::from_bytes(bytes)
}

fn noise(size: usize) -> Vec<u8> {
    let mut state = 0x1357_9bdfu32;
    let mut output = Vec::with_capacity(size);
    while output.len() < size {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        output.extend_from_slice(&state.to_le_bytes());
    }
    output.truncate(size);
    output
}

fn base() -> ArchiveInput {
    ArchiveInput::new(vec![ApplicationInput {
        application_id: APP,
        platform: Platform::Switch1,
        programs: vec![ProgramInput {
            program_id: PROGRAM,
            kind: ProgramKind::Primary,
            content_id: None,
            npdm: None,
        }],
        snapshots: vec![SnapshotInput {
            addon_title_id: None,
            native: Default::default(),
            program_id: Some(PROGRAM),
            snapshot_type: SnapshotType::Base,
            numeric_version: 0,
            content_id: None,
            build_id: Some([0x31; 32]),
            display_version: Some("1.0".into()),
            dependencies: vec![],
            metadata: Some(SnapshotMetadata {
                title: Some("Repack synthetic base".into()),
                publisher: Some("NX conformance".into()),
                icon: Some(MediaInput::from_bytes(
                    MediaType::PngIcon,
                    b"synthetic PNG body".to_vec(),
                )),
                nacp: None,
                localized: vec![LocalizedMetadata {
                    language: "en-us".into(),
                    title: Some("Repack synthetic base".into()),
                    icon: None,
                }],
                opaque: vec![],
            }),
            root: DirectoryInput::new(vec![EntryInput::file(
                "base.bin",
                input(vec![0x22; 64 * 1024]),
            )]),
        }],
    }])
}

fn additions(base_identity: [u8; 32]) -> ArchiveInput {
    let shared = vec![0x22; 64 * 1024];
    ArchiveInput::new(vec![ApplicationInput {
        application_id: APP,
        platform: Platform::Switch1,
        programs: vec![ProgramInput {
            program_id: PROGRAM,
            kind: ProgramKind::Primary,
            content_id: None,
            npdm: None,
        }],
        snapshots: vec![
            SnapshotInput {
                addon_title_id: None,
                native: Default::default(),
                program_id: Some(PROGRAM),
                snapshot_type: SnapshotType::Update,
                numeric_version: 1,
                content_id: None,
                build_id: Some([0x32; 32]),
                display_version: Some("1.1".into()),
                dependencies: vec![DependencyInput {
                    kind: DependencyKind::RequiredBase,
                    content_id: None,
                    target_snapshot_identity: base_identity,
                }],
                metadata: Some(SnapshotMetadata {
                    title: Some("Synthetic update".into()),
                    publisher: Some("NX update conformance".into()),
                    icon: Some(MediaInput::from_bytes(
                        MediaType::JpegIcon,
                        b"synthetic update JPEG".to_vec(),
                    )),
                    nacp: None,
                    localized: vec![LocalizedMetadata {
                        language: "fr".into(),
                        title: Some("Mise à jour synthétique".into()),
                        icon: Some(MediaInput::from_bytes(
                            MediaType::PngIcon,
                            b"synthetic localized PNG".to_vec(),
                        )),
                    }],
                    opaque: vec![],
                }),
                root: DirectoryInput::new(vec![
                    EntryInput::file("same.bin", input(shared)),
                    EntryInput::file("update.bin", input(vec![0x44; 256 * 1024])),
                ]),
            },
            SnapshotInput {
                addon_title_id: None,
                native: Default::default(),
                program_id: None,
                snapshot_type: SnapshotType::Dlc,
                numeric_version: 7,
                content_id: Some([0x55; 16]),
                build_id: None,
                display_version: None,
                dependencies: vec![],
                metadata: None,
                root: DirectoryInput::new(vec![EntryInput::file(
                    "dlc.bin",
                    input(noise(64 * 1024)),
                )]),
            },
        ],
    }])
}

fn run(source: &PathBuf, destination: &PathBuf, identity: [u8; 32]) -> Result<(), String> {
    let options = WriterOptions {
        chunk_size_bytes: 256 * 1024,
        ..WriterOptions::default()
    };
    let stats = repack_archive(
        source,
        destination,
        RepackPlan {
            remove_snapshots: vec![],
            additions: additions(identity),
            metadata_sources: vec![MetadataSourceSelection {
                platform: Platform::Switch1,
                application_id: APP,
                snapshot_identity: identity,
            }],
        },
        &options,
        IntegrityPolicy::Strict,
    )
    .map_err(|error| error.to_string())?;
    if stats.added_snapshots != 2 || stats.added_blocks_reused != 1 {
        return Err(format!("unexpected repack accounting: {stats:?}"));
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let first = arguments
        .next()
        .ok_or("usage: repack_conformance <output-directory> | --inspect <archive.nx>")?;
    if first == "--inspect" {
        let path = PathBuf::from(arguments.next().ok_or("missing archive path")?);
        let summary = inspect_archive(&path, &WriterOptions::default(), IntegrityPolicy::Fast)?;
        println!(
            "PASS Rust repack parser accepted {} applications",
            summary.applications.len()
        );
        return Ok(());
    }
    let output = PathBuf::from(first);
    fs::create_dir_all(&output)?;
    let source = output.join("source.nx");
    let edited = output.join("repacked.nx");
    let repeated = output.join("repacked_repeat.nx");
    let selected_update = output.join("metadata_selected_update.nx");
    let fallback = output.join("metadata_fallback.nx");
    let source_options = WriterOptions {
        chunk_size_bytes: 64 * 1024,
        ..WriterOptions::default()
    };
    write_archive(base(), &source, &source_options)?;
    let identity = inspect_archive(&source, &source_options, IntegrityPolicy::Strict)?
        .applications
        .into_iter()
        .next()
        .and_then(|app| app.snapshots.into_iter().next())
        .ok_or("source base identity missing")?
        .identity;
    run(&source, &edited, identity).map_err(std::io::Error::other)?;
    run(&source, &repeated, identity).map_err(std::io::Error::other)?;
    let summary = inspect_archive(&edited, &source_options, IntegrityPolicy::Strict)?;
    let update_identity = summary.applications[0]
        .snapshots
        .iter()
        .find(|snapshot| snapshot.snapshot_type == SnapshotType::Update)
        .ok_or("update snapshot identity missing")?
        .identity;
    repack_archive(
        &edited,
        &selected_update,
        RepackPlan {
            remove_snapshots: vec![],
            additions: ArchiveInput::new(vec![]),
            metadata_sources: vec![MetadataSourceSelection {
                platform: Platform::Switch1,
                application_id: APP,
                snapshot_identity: update_identity,
            }],
        },
        &source_options,
        IntegrityPolicy::Strict,
    )?;
    repack_archive(
        &selected_update,
        &fallback,
        RepackPlan {
            remove_snapshots: vec![update_identity],
            additions: ArchiveInput::new(vec![]),
            metadata_sources: vec![],
        },
        &source_options,
        IntegrityPolicy::Strict,
    )?;
    Ok(())
}
