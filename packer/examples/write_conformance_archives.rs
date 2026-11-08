use nxformat_packer::{
    write_archive, ApplicationInput, ArchiveInput, CompressionMode, DependencyInput,
    DependencyKind, DirectoryInput, EntryInput, FileInput, LocalizedMetadata, MediaInput,
    MediaType, Platform, ProgramInput, ProgramKind, SnapshotInput, SnapshotMetadata, SnapshotType,
    WriterOptions,
};
use std::io::{self, Read};
use std::path::PathBuf;

struct PatternReader {
    remaining: u64,
    position: u64,
}
impl Read for PatternReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = (self.remaining.min(out.len() as u64)) as usize;
        for dst in &mut out[..n] {
            *dst = ((self.position.wrapping_mul(17).wrapping_add(11)) & 0xff) as u8;
            self.position += 1;
        }
        self.remaining -= n as u64;
        Ok(n)
    }
}

struct NoiseReader {
    remaining: u64,
    state: u32,
    word: [u8; 4],
    word_pos: usize,
}
impl Read for NoiseReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = self.remaining.min(out.len() as u64) as usize;
        for dst in &mut out[..n] {
            if self.word_pos == 4 {
                let mut x = self.state;
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                self.state = x;
                self.word = x.to_le_bytes();
                self.word_pos = 0;
            }
            *dst = self.word[self.word_pos];
            self.word_pos += 1;
        }
        self.remaining -= n as u64;
        Ok(n)
    }
}

fn snapshot(
    program_id: Option<u64>,
    kind: SnapshotType,
    version: u32,
    root: DirectoryInput,
) -> SnapshotInput {
    SnapshotInput {
        addon_title_id: None,
        native: Default::default(),
        program_id,
        snapshot_type: kind,
        numeric_version: version,
        content_id: None,
        build_id: None,
        display_version: None,
        dependencies: Vec::new(),
        metadata: None,
        root,
    }
}

fn raw_case() -> ArchiveInput {
    let pid = 0x0100_0000_0000_1001;
    let big = 700_123u64;
    let tree = DirectoryInput::new(vec![
        EntryInput::directory(
            "folder",
            DirectoryInput::new(vec![EntryInput::file(
                "big.bin",
                FileInput::new(
                    big,
                    PatternReader {
                        remaining: big,
                        position: 0,
                    },
                ),
            )]),
        ),
        EntryInput::file("empty.bin", FileInput::from_bytes(Vec::new())),
        EntryInput::file(
            "note.txt",
            FileInput::from_bytes(b"NX writer stream test".to_vec()),
        ),
    ]);
    ArchiveInput::new(vec![ApplicationInput {
        application_id: 0x0100_0000_0000_2001,
        platform: Platform::Switch1,
        programs: vec![ProgramInput {
            program_id: pid,
            kind: ProgramKind::Primary,
            content_id: Some([0x21; 16]),
            npdm: None,
        }],
        snapshots: vec![snapshot(Some(pid), SnapshotType::Base, 1, tree)],
    }])
}

fn metadata_case() -> ArchiveInput {
    let pid = 0x0100_0000_0000_3001;
    let shared = b"stable file across complete snapshots".to_vec();
    let base_tree = DirectoryInput::new(vec![
        EntryInput::file(
            "removed.bin",
            FileInput::from_bytes(b"removed in update".to_vec()),
        ),
        EntryInput::file("shared.bin", FileInput::from_bytes(shared.clone())),
    ]);
    let update_tree = DirectoryInput::new(vec![
        EntryInput::file(
            "new.bin",
            FileInput::from_bytes(b"added in update".to_vec()),
        ),
        EntryInput::file("shared.bin", FileInput::from_bytes(shared)),
    ]);
    let base_metadata = SnapshotMetadata {
        title: Some("Original native title".to_owned()),
        publisher: Some("Original publisher".to_owned()),
        icon: Some(MediaInput::from_bytes(
            MediaType::PngIcon,
            b"original PNG bytes".to_vec(),
        )),
        nacp: Some(MediaInput::from_bytes(
            MediaType::Nacp,
            b"synthetic base NACP".to_vec(),
        )),
        localized: vec![LocalizedMetadata {
            language: "ja".to_owned(),
            title: Some("元のタイトル".to_owned()),
            icon: Some(MediaInput::from_bytes(
                MediaType::PngIcon,
                b"original PNG bytes".to_vec(),
            )),
        }],
        opaque: vec![MediaInput::from_bytes(
            MediaType::Opaque,
            b"base opaque source metadata".to_vec(),
        )],
    };
    let update_metadata = SnapshotMetadata {
        title: Some("Current native title".to_owned()),
        publisher: Some("Current publisher".to_owned()),
        icon: Some(MediaInput::from_bytes(
            MediaType::JpegIcon,
            b"current JPEG bytes".to_vec(),
        )),
        nacp: Some(MediaInput::from_bytes(
            MediaType::Nacp,
            b"synthetic update NACP".to_vec(),
        )),
        localized: vec![LocalizedMetadata {
            language: "fr".to_owned(),
            title: Some("Titre actuel".to_owned()),
            icon: None,
        }],
        opaque: Vec::new(),
    };
    let base = SnapshotInput {
        addon_title_id: None,
        native: nxformat_packer::SnapshotNativeMetadata {
            npdm: Some(MediaInput::from_bytes(
                MediaType::Npdm,
                b"base NPDM".to_vec(),
            )),
            required_firmware: Some(100),
        },
        program_id: Some(pid),
        snapshot_type: SnapshotType::Base,
        numeric_version: 10,
        content_id: Some([0x10; 16]),
        build_id: Some([0x30; 32]),
        display_version: Some("1.0.0".to_owned()),
        dependencies: Vec::new(),
        metadata: Some(base_metadata),
        root: base_tree,
    };
    let mut update = SnapshotInput {
        addon_title_id: None,
        native: nxformat_packer::SnapshotNativeMetadata {
            npdm: Some(MediaInput::from_bytes(
                MediaType::Npdm,
                b"update NPDM".to_vec(),
            )),
            required_firmware: Some(200),
        },
        program_id: Some(pid),
        snapshot_type: SnapshotType::Update,
        numeric_version: 11,
        content_id: Some([0x11; 16]),
        build_id: Some([0x31; 32]),
        display_version: Some("1.1.0".to_owned()),
        dependencies: Vec::new(),
        metadata: Some(update_metadata),
        root: update_tree,
    };
    update.dependencies.push(DependencyInput {
        kind: DependencyKind::Recommends,
        content_id: None,
        target_snapshot_identity: [0x42; 32],
    });
    let dlc = SnapshotInput {
        addon_title_id: None,
        native: Default::default(),
        program_id: None,
        snapshot_type: SnapshotType::Dlc,
        numeric_version: 1,
        content_id: Some([0x13; 16]),
        build_id: Some([0x33; 32]),
        display_version: None,
        dependencies: Vec::new(),
        metadata: None,
        root: DirectoryInput::empty(),
    };
    ArchiveInput::new(vec![ApplicationInput {
        application_id: 0x0100_0000_0000_3000,
        platform: Platform::Switch2,
        programs: vec![ProgramInput {
            program_id: pid,
            kind: ProgramKind::Primary,
            content_id: Some([0x20; 16]),
            npdm: None,
        }],
        snapshots: vec![update, dlc, base],
    }])
}

fn empty_case() -> ArchiveInput {
    let pid = 0x0100_0000_0000_4001;
    ArchiveInput::new(vec![ApplicationInput {
        application_id: 0x0100_0000_0000_4000,
        platform: Platform::Unknown,
        programs: vec![ProgramInput {
            program_id: pid,
            kind: ProgramKind::Unknown,
            content_id: None,
            npdm: None,
        }],
        snapshots: vec![snapshot(
            Some(pid),
            SnapshotType::Unknown,
            0,
            DirectoryInput::empty(),
        )],
    }])
}

fn dlc_only_case() -> ArchiveInput {
    ArchiveInput::new(vec![ApplicationInput {
        application_id: 0x0100_0000_0000_5000,
        platform: Platform::Unknown,
        programs: Vec::new(),
        snapshots: vec![SnapshotInput {
            addon_title_id: None,
            native: Default::default(),
            program_id: None,
            snapshot_type: SnapshotType::Dlc,
            numeric_version: 1,
            content_id: Some([0x51; 16]),
            build_id: Some([0x52; 32]),
            display_version: None,
            dependencies: Vec::new(),
            metadata: None,
            root: DirectoryInput::new(vec![EntryInput::file(
                "dlc.bin",
                FileInput::from_bytes(b"unassociated DLC tree".to_vec()),
            )]),
        }],
    }])
}

fn multiple_programs_case() -> ArchiveInput {
    let first_id = 0x0100_0000_0000_6001;
    let second_id = 0x0100_0000_0000_6002;
    ArchiveInput::new(vec![ApplicationInput {
        application_id: 0x0100_0000_0000_6000,
        platform: Platform::Switch1,
        // Deliberately not sorted: the writer emits canonical Catalog order.
        programs: vec![
            ProgramInput {
                program_id: second_id,
                kind: ProgramKind::Auxiliary,
                content_id: Some([0x62; 16]),
                npdm: None,
            },
            ProgramInput {
                program_id: first_id,
                kind: ProgramKind::Primary,
                content_id: Some([0x61; 16]),
                npdm: None,
            },
        ],
        snapshots: vec![
            snapshot(
                Some(second_id),
                SnapshotType::Base,
                1,
                DirectoryInput::new(vec![
                    EntryInput::file(
                        "second.bin",
                        FileInput::from_bytes(b"second executable tree".to_vec()),
                    ),
                    EntryInput::file(
                        "shared.bin",
                        FileInput::from_bytes(b"first executable tree".to_vec()),
                    ),
                ]),
            ),
            snapshot(
                Some(first_id),
                SnapshotType::Base,
                1,
                DirectoryInput::new(vec![EntryInput::file(
                    "first.bin",
                    FileInput::from_bytes(b"first executable tree".to_vec()),
                )]),
            ),
        ],
    }])
}

fn dedup_case() -> ArchiveInput {
    const CHUNK: usize = 256 * 1024;
    let a = vec![0xa1; CHUNK];
    let b = vec![0xb2; CHUNK];
    let c = vec![0xc3; CHUNK];
    let d = vec![0xd4; CHUNK];
    let tail = vec![0xe5; 137];
    let two = |first: &[u8], second: &[u8]| {
        let mut bytes = Vec::with_capacity(first.len() + second.len());
        bytes.extend_from_slice(first);
        bytes.extend_from_slice(second);
        bytes
    };
    let first_program = 0x0100_0000_0000_8001;
    let mut first_app = ApplicationInput {
        application_id: 0x0100_0000_0000_8000,
        platform: Platform::Switch1,
        programs: vec![ProgramInput {
            program_id: first_program,
            kind: ProgramKind::Primary,
            content_id: None,
            npdm: None,
        }],
        snapshots: vec![
            snapshot(
                Some(first_program),
                SnapshotType::Base,
                1,
                DirectoryInput::new(vec![
                    EntryInput::file("a.bin", FileInput::from_bytes(two(&a, &b))),
                    EntryInput::file("duplicate.bin", FileInput::from_bytes(two(&a, &b))),
                    EntryInput::file("changed.bin", FileInput::from_bytes(two(&a, &c))),
                    EntryInput::file("empty.bin", FileInput::from_bytes(Vec::new())),
                ]),
            ),
            snapshot(
                Some(first_program),
                SnapshotType::Update,
                2,
                DirectoryInput::new(vec![
                    EntryInput::file("a.bin", FileInput::from_bytes(two(&a, &b))),
                    EntryInput::file("changed.bin", FileInput::from_bytes(two(&a, &d))),
                    EntryInput::file("copy.bin", FileInput::from_bytes(two(&a, &b))),
                    EntryInput::file("tail.bin", FileInput::from_bytes(tail.clone())),
                ]),
            ),
        ],
    };
    // Keep the constructor explicit about the same Program association across versions.
    first_app.snapshots[0].content_id = Some([0x81; 16]);
    first_app.snapshots[1].content_id = Some([0x82; 16]);
    let second_program = 0x0100_0000_0000_9001;
    let second_app = ApplicationInput {
        application_id: 0x0100_0000_0000_9000,
        platform: Platform::Switch2,
        programs: vec![ProgramInput {
            program_id: second_program,
            kind: ProgramKind::Primary,
            content_id: None,
            npdm: None,
        }],
        snapshots: vec![SnapshotInput {
            addon_title_id: None,
            native: Default::default(),
            program_id: Some(second_program),
            snapshot_type: SnapshotType::Base,
            numeric_version: 1,
            content_id: Some([0x91; 16]),
            build_id: None,
            display_version: None,
            dependencies: Vec::new(),
            metadata: None,
            root: DirectoryInput::new(vec![EntryInput::file(
                "cross.bin",
                FileInput::from_bytes(a),
            )]),
        }],
    };
    ArchiveInput::new(vec![first_app, second_app])
}

fn removed_case(kind: SnapshotType) -> ArchiveInput {
    let mut input = metadata_case();
    input.applications[0]
        .snapshots
        .retain(|s| s.snapshot_type != kind);
    input
}
fn added_dlc_case() -> ArchiveInput {
    let mut input = metadata_case();
    let mut dlc = snapshot(
        None,
        SnapshotType::Dlc,
        2,
        DirectoryInput::new(vec![EntryInput::file(
            "extra.bin",
            FileInput::from_bytes(b"new DLC content".to_vec()),
        )]),
    );
    dlc.content_id = Some([0x14; 16]);
    input.applications[0].snapshots.push(dlc);
    input
}
fn added_update_case() -> ArchiveInput {
    let mut input = metadata_case();
    let mut update = snapshot(
        Some(0x0100_0000_0000_3001),
        SnapshotType::Update,
        12,
        DirectoryInput::new(vec![
            EntryInput::file(
                "shared.bin",
                FileInput::from_bytes(b"stable file across complete snapshots".to_vec()),
            ),
            EntryInput::file(
                "future.bin",
                FileInput::from_bytes(b"third version".to_vec()),
            ),
        ]),
    );
    update.content_id = Some([0x12; 16]);
    update.native = nxformat_packer::SnapshotNativeMetadata {
        npdm: Some(MediaInput::from_bytes(
            MediaType::Npdm,
            b"third NPDM".to_vec(),
        )),
        required_firmware: Some(300),
    };
    update.metadata = Some(SnapshotMetadata {
        title: Some("Third version title".into()),
        publisher: None,
        icon: None,
        nacp: None,
        localized: vec![],
        opaque: vec![],
    });
    input.applications[0].snapshots.push(update);
    input
}

fn codec_case(noise: bool) -> ArchiveInput {
    let chunk = 256 * 1024u64;
    let logical_size = chunk * 2 + chunk / 2;
    let mut entries = vec![EntryInput::file(
        "compressible.bin",
        FileInput::from_bytes(vec![0x5a; logical_size as usize]),
    )];
    if noise {
        entries.push(EntryInput::file(
            "incompressible.bin",
            FileInput::new(
                chunk,
                NoiseReader {
                    remaining: chunk,
                    state: 0x1357_9bdf,
                    word: [0; 4],
                    word_pos: 4,
                },
            ),
        ));
    }
    ArchiveInput::new(vec![ApplicationInput {
        application_id: 0x0100_0000_0000_8000,
        platform: Platform::Unknown,
        programs: vec![ProgramInput {
            program_id: 0x0100_0000_0000_8001,
            kind: ProgramKind::Unknown,
            content_id: None,
            npdm: None,
        }],
        snapshots: vec![snapshot(
            Some(0x0100_0000_0000_8001),
            SnapshotType::Base,
            1,
            DirectoryInput::new(entries),
        )],
    }])
}

fn chunk_size_case(chunk_size: u32) -> ArchiveInput {
    let size = chunk_size as usize * 2 + 37;
    let root = DirectoryInput::new(vec![EntryInput::file(
        "chunk.bin",
        FileInput::from_bytes(vec![0x6b; size]),
    )]);
    ArchiveInput::new(vec![ApplicationInput {
        application_id: 0x0100_0000_0000_8100 + chunk_size as u64,
        platform: Platform::Unknown,
        programs: vec![ProgramInput {
            program_id: 0x0100_0000_0000_8200 + chunk_size as u64,
            kind: ProgramKind::Unknown,
            content_id: None,
            npdm: None,
        }],
        snapshots: vec![snapshot(
            Some(0x0100_0000_0000_8200 + chunk_size as u64),
            SnapshotType::Base,
            1,
            root,
        )],
    }])
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).ok_or("expected output directory")?);
    std::fs::create_dir_all(&out)?;
    let mut cases = vec![
        ("empty.nx".to_owned(), empty_case(), golden_options()),
        (
            "raw_multiblock.nx".to_owned(),
            raw_case(),
            WriterOptions {
                compression_mode: CompressionMode::RawOnly,
                ..golden_options()
            },
        ),
        (
            "snapshot_metadata.nx".to_owned(),
            metadata_case(),
            golden_options(),
        ),
        ("dlc_only.nx".to_owned(), dlc_only_case(), golden_options()),
        (
            "multiple_programs.nx".to_owned(),
            multiple_programs_case(),
            golden_options(),
        ),
        ("global_dedup.nx".to_owned(), dedup_case(), golden_options()),
        (
            "removed_update.nx".to_owned(),
            removed_case(SnapshotType::Update),
            golden_options(),
        ),
        (
            "removed_dlc.nx".to_owned(),
            removed_case(SnapshotType::Dlc),
            golden_options(),
        ),
        (
            "removed_base.nx".to_owned(),
            removed_case(SnapshotType::Base),
            golden_options(),
        ),
        (
            "added_update.nx".to_owned(),
            added_update_case(),
            golden_options(),
        ),
        (
            "added_dlc.nx".to_owned(),
            added_dlc_case(),
            golden_options(),
        ),
        (
            "zstd_only.nx".to_owned(),
            codec_case(false),
            golden_options(),
        ),
        (
            "mixed_codecs.nx".to_owned(),
            codec_case(true),
            golden_options(),
        ),
        (
            "zstd_no_readback.nx".to_owned(),
            codec_case(false),
            WriterOptions {
                verify_output_payloads: false,
                ..golden_options()
            },
        ),
    ];
    for chunk_size in [64 * 1024, 128 * 1024, 256 * 1024, 512 * 1024, 1024 * 1024] {
        cases.push((
            format!("chunk_{}k.nx", chunk_size / 1024),
            chunk_size_case(chunk_size),
            WriterOptions {
                chunk_size_bytes: chunk_size,
                ..golden_options()
            },
        ));
    }
    for (name, input, options) in cases {
        let stats = write_archive(input, out.join(&name), &options)?;
        println!(
            "{name}: input={} archive={} files={} blocks={} raw={} zstd={} logical_stage={} stored_stage={} elapsed_us={} input_Bps={} accounted_memory={} temp_disk={}",
            stats.input_bytes,
            stats.archive_bytes,
            stats.file_records,
            stats.block_count,
            stats.raw_block_count,
            stats.zstd_block_count,
            stats.staged_logical_block_bytes,
            stats.staged_stored_block_bytes,
            stats.elapsed.as_micros(),
            stats.input_bytes_per_second,
            stats.accounted_memory_bytes,
            stats.peak_temporary_disk_bytes
        );
    }
    Ok(())
}

/// Golden layouts predate content-based chunk sizing; keep their chunks fixed.
fn golden_options() -> WriterOptions {
    WriterOptions {
        compressed_chunk_bytes: 0,
        ..WriterOptions::default()
    }
}
