use super::*;
use crate::import::{structures::NpdmProfile, synthetic::npdm, synthetic_nup::*};
const APP: u64 = 0x0100_0000_1234_0000;
const AUX: u64 = 0x0400_0000_1234_0001;
fn work(name: &str) -> PathBuf {
    super::super::tests::work(name)
}
fn fixture() -> Vec<u8> {
    let mut primary = base(APP);
    primary.exefs.as_mut().unwrap().iter_mut().for_each(|n| {
        if let Node::File("main.npdm", b) = n {
            *b = npdm(APP, NpdmProfile::MetaAcid2048V1)
        }
    });
    build_with_extras(
        &[base(AUX), primary, addon(APP), update(AUX, 7, 0)],
        true,
        &[(
            1,
            vec![Node::Dir(
                "legal",
                vec![Node::File("notice.txt", b"preserve me".to_vec())],
            )],
        )],
    )
}
fn map_text(bytes: &[u8]) -> String {
    let digest = bytes[24..56]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    format!("NXNUPMAP\t1\t{digest}\nNXIMPORT\t1\n\
snapshot\tmain\tpackage-1\tswitch2\t{APP:016x}\t{APP:016x}\tprimary\tbase\t0\tcomplete\tmaterialized-v1\tmeta-acid2048-v1\n\
snapshot\tsecondary\tpackage-0\tswitch2\t{APP:016x}\t{AUX:016x}\tauxiliary\tbase\t0\tcomplete\tmaterialized-v1\tmeta-acid3072-v1\n\
snapshot\tdlc\tpackage-2\tswitch2\t{APP:016x}\t-\tnone\tdlc\t0\tcomplete\tmaterialized-v1\tnone\n\
addon\tdlc\t{:016x}\n\
snapshot\tpatch\tpackage-3\tswitch2\t{APP:016x}\t{AUX:016x}\tauxiliary\tupdate\t7\tcomplete\tmaterialized-v1\tmeta-acid3072-v1\n\
dependency\tpatch\trequired-base\tsecondary\n",APP+0x1001)
}

#[test]
fn mapped_mixed_programs_update_dlc_and_extras_round_trip() {
    let dir = work("compat-roundtrip");
    let input = dir.join("game.nup");
    let mapping = dir.join("game.nxnupmap");
    let bytes = fixture();
    fs::write(&input, &bytes).unwrap();
    fs::write(&mapping, map_text(&bytes)).unwrap();
    let limits = ImportLimits::default();
    let token = crate::CancellationToken::default();
    assert!(inspect_nup(&input, &limits, &token).is_err());
    assert!(stage_nup(
        &input,
        &dir,
        &NupOptions {
            platform: Some(Platform::Switch2),
            ..Default::default()
        },
        &limits,
        &token
    )
    .is_err());
    let info = inspect_nup_with_mapping(&input, &mapping, &limits, &token).unwrap();
    assert_eq!(info.application_id, APP);
    assert_eq!(info.platform, Platform::Switch2);
    assert_eq!(info.packages.len(), 4);
    for stream in [false, true] {
        let output = dir.join(format!("{stream}.nx"));
        let opts = WriterOptions::default();
        convert_nup(
            &input,
            &output,
            &NupOptions {
                mapping: Some(mapping.clone()),
                platform: Some(Platform::Switch2),
                stream,
                ..Default::default()
            },
            &limits,
            &opts,
        )
        .unwrap();
        let summary =
            crate::inspect_archive(&output, &opts, crate::IntegrityPolicy::Strict).unwrap();
        assert_eq!(summary.applications.len(), 1);
        let app = &summary.applications[0];
        assert_eq!(app.application_id, APP);
        assert_eq!(app.snapshots.len(), 4);
        assert!(app
            .programs
            .iter()
            .any(|p| p.program_id == APP && p.kind == ProgramKind::Primary));
        assert!(app
            .programs
            .iter()
            .any(|p| p.program_id == AUX && p.kind == ProgramKind::Auxiliary));
        assert!(app
            .snapshots
            .iter()
            .any(|s| s.snapshot_type == SnapshotType::Dlc
                && s.addon_title_id == Some(APP + 0x1001)
                && s.program_id.is_none()));
        let patch = app
            .snapshots
            .iter()
            .find(|s| s.snapshot_type == SnapshotType::Update)
            .unwrap();
        let aux = app
            .snapshots
            .iter()
            .find(|s| s.snapshot_type == SnapshotType::Base && s.program_id == Some(AUX))
            .unwrap();
        assert!(patch
            .dependencies
            .iter()
            .any(|d| d.target_snapshot_identity == aux.identity));
        let check =
            check_nup_with_mapping(&input, &output, false, &mapping, &limits, &opts).unwrap();
        assert_eq!(check.snapshots, 4);
        assert!(check.files_read > 0);
        if stream {
            let extracted = dir.join("extracted");
            crate::cli::run(
                vec![
                    "extract".into(),
                    output.to_string_lossy().into_owned(),
                    extracted.to_string_lossy().into_owned(),
                    "--no-restore".into(),
                ],
                None,
                None,
            )
            .unwrap();
            let manifest = DeclarationManifest::read(
                fs::read(extracted.join("extracted.nximport"))
                    .unwrap()
                    .as_slice(),
                &limits,
            )
            .unwrap();
            let main = manifest
                .snapshots
                .iter()
                .find(|s| s.program_id == Some(APP))
                .unwrap();
            let root = extracted.join(&main.root);
            assert_eq!(
                fs::read(root.join("data/__nup_root__/legal/notice.txt")).unwrap(),
                b"preserve me"
            );
            assert!(fs::read_to_string(root.join("data/__nup_mapping__.txt"))
                .unwrap()
                .contains("TOC SHA256:"));
        }
    }
    assert_eq!(fs::read(&input).unwrap(), bytes);
}

#[test]
fn mapped_ambiguities_stale_hash_wrong_profile_and_dependencies_reject() {
    let dir = work("compat-reject");
    let input = dir.join("game.nup");
    let mapping = dir.join("game.nxnupmap");
    let bytes = fixture();
    fs::write(&input, &bytes).unwrap();
    let valid = map_text(&bytes);
    for text in [
        valid.replacen("NXNUPMAP\t1\t", "NXNUPMAP\t1\t0", 1),
        valid.replace("package-0", "package-1"),
        valid.replace("meta-acid2048-v1", "meta-acid3072-v1"),
        valid.replace("required-base\tsecondary", "required-base\tmain"),
        valid.replace("auxiliary\tbase", "primary\tbase"),
        valid.replace(&format!("{AUX:016x}"), "0400000012340002"),
    ] {
        fs::write(&mapping, text).unwrap();
        assert!(inspect_nup_with_mapping(
            &input,
            &mapping,
            &ImportLimits::default(),
            &crate::CancellationToken::default()
        )
        .is_err());
    }
    fs::write(&mapping, &valid).unwrap();
    inspect_nup_with_mapping(
        &input,
        &mapping,
        &ImportLimits::default(),
        &crate::CancellationToken::default(),
    )
    .unwrap();
    fs::write(&mapping, valid.replace("NXNUPMAP\t1\t", "NXNUPMAP\t1\t0")).unwrap();
    let output = dir.join("stale.nx");
    assert!(convert_nup(
        &input,
        &output,
        &NupOptions {
            platform: Some(Platform::Switch2),
            mapping: Some(mapping),
            stream: true,
            ..Default::default()
        },
        &ImportLimits::default(),
        &WriterOptions::default()
    )
    .is_err());
    assert!(!output.exists());
}

fn cnmt() -> Vec<u8> {
    let mut b = vec![0; 0x30 + 2 * 0x38];
    b[..8].copy_from_slice(&APP.to_le_bytes());
    b[0xc] = 0x80;
    b[0xd] = 1;
    b[0xe] = 0x10;
    b[0x10] = 2;
    for i in 0..2 {
        b[0x30 + i * 0x38 + 0x36] = 1;
        b[0x30 + i * 0x38 + 0x37] = i as u8;
    }
    b
}

#[test]
fn cnmt_program_set_rejects_missing_extra_unrelated_and_wrong_platform() {
    let dir = work("compat-cnmt");
    let input = dir.join("game.nup");
    let mapping = dir.join("game.nxnupmap");
    for case in 0..8 {
        let other = match case {
            2 => AUX + 1,
            3 => AUX + 0x10000,
            _ => AUX,
        };
        let mut primary = base(APP);
        primary.exefs.as_mut().unwrap().iter_mut().for_each(|n| {
            if let Node::File("main.npdm", b) = n {
                *b = npdm(APP, NpdmProfile::MetaAcid2048V1)
            }
        });
        let mut packages = vec![base(other), primary, addon(APP), update(other, 7, 0)];
        let mut meta = cnmt();
        if case == 4 {
            meta[0xd] = 0;
        }
        if case == 5 {
            meta[8] = 1;
        }
        if case == 6 {
            meta[0xc] = 0x81;
        }
        if case == 7 {
            meta[0x30 + 0x38 + 0x37] = 0;
        }
        let bytes = build_with_extras(
            &packages,
            true,
            &[(
                1,
                vec![Node::Dir(
                    "meta",
                    vec![Node::File("Application.cnmt", meta.clone())],
                )],
            )],
        );
        let mut text = map_text(&bytes).replace(&format!("{AUX:016x}"), &format!("{other:016x}"));
        let bytes = if case == 1 {
            packages.remove(0);
            packages.pop();
            let bytes = build_with_extras(
                &packages,
                true,
                &[(
                    0,
                    vec![Node::Dir(
                        "meta",
                        vec![Node::File("Application.cnmt", meta)],
                    )],
                )],
            );
            text = map_text(&bytes)
                .lines()
                .filter(|l| {
                    !l.starts_with("snapshot\tsecondary\t")
                        && !l.starts_with("snapshot\tpatch\t")
                        && !l.starts_with("dependency\t")
                })
                .collect::<Vec<_>>()
                .join("\n")
                .replace("package-1", "package-0")
                .replace("package-2", "package-1")
                + "\n";
            bytes
        } else {
            bytes
        };
        fs::write(&input, bytes).unwrap();
        fs::write(&mapping, text).unwrap();
        let result = inspect_nup_with_mapping(
            &input,
            &mapping,
            &ImportLimits::default(),
            &crate::CancellationToken::default(),
        );
        if case == 0 {
            result.unwrap();
        } else {
            assert!(result.is_err(), "case {case}");
        }
    }
}

#[test]
fn mapped_stream_still_rejects_corrupt_source_payloads() {
    let dir = work("compat-corrupt");
    let input = dir.join("game.nup");
    let mapping = dir.join("game.nxnupmap");
    let mut bytes = fixture();
    let o = WriterOptions::default();
    fs::write(&input, &bytes).unwrap();
    fs::write(&mapping, map_text(&bytes)).unwrap();
    let (toc, _) = read_toc(&mut File::open(&input).unwrap(), &ImportLimits::default()).unwrap();
    let f = toc.files.iter().find(|f| f.name == "common.bin").unwrap();
    bytes[f.extent_offset as usize] ^= 0x7f;
    fs::write(&input, bytes).unwrap();
    let output = dir.join("bad.nx");
    assert!(convert_nup(
        &input,
        &output,
        &NupOptions {
            mapping: Some(mapping),
            platform: Some(Platform::Switch2),
            stream: true,
            ..Default::default()
        },
        &ImportLimits::default(),
        &o
    )
    .is_err());
    assert!(!output.exists());
}

fn generation_fixture(ambiguous: bool) -> Vec<u8> {
    fn meta(kind: u8, title: u64, version: u32, offsets: &[u8]) -> Vec<u8> {
        let ext = if kind == 0x81 { 0x18 } else { 0x10 };
        let mut b = vec![0; 0x20 + ext + offsets.len() * 0x38];
        b[..8].copy_from_slice(&title.to_le_bytes());
        b[8..12].copy_from_slice(&version.to_le_bytes());
        b[0xc] = kind;
        b[0xd] = 1;
        b[0xe] = ext as u8;
        b[0x10..0x12].copy_from_slice(&(offsets.len() as u16).to_le_bytes());
        if kind != 0x80 {
            b[0x20..0x28].copy_from_slice(&APP.to_le_bytes());
        }
        for (i, offset) in offsets.iter().enumerate() {
            b[0x20 + ext + i * 0x38 + 0x36] = 1;
            b[0x20 + ext + i * 0x38 + 0x37] = *offset;
        }
        b
    }
    let mut primary = base(APP);
    for n in primary.exefs.iter_mut().flatten() {
        if let Node::File("main.npdm", b) = n {
            *b = npdm(APP, NpdmProfile::MetaAcid2048V1);
        }
    }
    let mut main_update = update(APP, 7, 0);
    for n in main_update.exefs.iter_mut().flatten() {
        if let Node::File("main.npdm", b) = n {
            *b = npdm(APP, NpdmProfile::MetaAcid2048V1);
        }
    }
    let mut extras = Vec::new();
    for i in 0..5 {
        if ambiguous && i == 0 {
            continue;
        }
        let bytes = match i {
            0 | 1 => meta(0x80, APP, 0, &[0, 1]),
            2 => meta(0x82, APP + 0x1001, 0, &[]),
            _ => meta(0x81, APP + 0x800, 7, &[0, 1]),
        };
        extras.push((
            i,
            vec![Node::Dir("meta", vec![Node::File("content.cnmt", bytes)])],
        ));
    }
    build_with_extras(
        &[
            base(AUX),
            primary,
            addon(APP),
            update(AUX, 7, 0),
            main_update,
        ],
        true,
        &extras,
    )
}

#[test]
fn generated_mapping_preserves_programs_updates_dlc_and_converts() {
    let dir = work("generated");
    let input = dir.join("source.nup");
    let mapping = dir.join("generated.nxnupmap");
    let output = dir.join("generated.nx");
    let bytes = generation_fixture(false);
    fs::write(&input, &bytes).unwrap();
    let limits = ImportLimits::default();
    let token = crate::CancellationToken::default();
    let text = generate_nup_mapping(&input, &limits, &token).unwrap();
    assert!(text.contains("primary\tbase\t0"));
    assert!(text.contains("auxiliary\tupdate\t7"));
    assert!(text.contains("dependency\tpkg-3\trequired-base\tpkg-0"));
    assert!(text.contains("dependency\tpkg-4\trequired-base\tpkg-1"));
    let args = vec![
        "map-nup".into(),
        input.to_string_lossy().into_owned(),
        mapping.to_string_lossy().into_owned(),
    ];
    assert!(crate::cli::run(args.clone(), None, None).is_err());
    assert!(!mapping.exists());
    let mut args = args;
    args.push("--assume-complete".into());
    crate::cli::run(args.clone(), None, None).unwrap();
    assert_eq!(fs::read_to_string(&mapping).unwrap(), text);
    assert!(crate::cli::run(args, None, None).is_err());
    assert_eq!(fs::read_to_string(&mapping).unwrap(), text);
    convert_nup(
        &input,
        &output,
        &NupOptions {
            mapping: Some(mapping.clone()),
            platform: Some(Platform::Switch2),
            stream: true,
            ..Default::default()
        },
        &limits,
        &WriterOptions::default(),
    )
    .unwrap();
    let summary = crate::inspect_archive(
        &output,
        &WriterOptions::default(),
        crate::IntegrityPolicy::Strict,
    )
    .unwrap();
    assert_eq!(summary.applications[0].programs.len(), 2);
    assert_eq!(summary.applications[0].snapshots.len(), 5);
    assert_eq!(
        check_nup_with_mapping(
            &input,
            &output,
            false,
            &mapping,
            &limits,
            &WriterOptions::default()
        )
        .unwrap()
        .snapshots,
        5
    );
    assert_eq!(fs::read(&input).unwrap(), bytes);
}

#[test]
fn generation_rejects_missing_ambiguous_cancelled_and_over_budget() {
    let dir = work("generation-reject");
    let input = dir.join("source.nup");
    let token = crate::CancellationToken::default();
    let limits = ImportLimits::default();
    let mut conflicting = cnmt();
    conflicting[0xd] = 0;
    let conflict = build_with_extras(
        &[base(AUX), base(APP)],
        true,
        &[
            (
                0,
                vec![Node::Dir(
                    "meta",
                    vec![Node::File("Application.cnmt", conflicting)],
                )],
            ),
            (
                1,
                vec![Node::Dir(
                    "meta",
                    vec![Node::File("Application.cnmt", cnmt())],
                )],
            ),
        ],
    );
    for bytes in [fixture(), generation_fixture(true), conflict] {
        fs::write(&input, bytes).unwrap();
        assert!(generate_nup_mapping(&input, &limits, &token).is_err());
    }
    fs::write(&input, generation_fixture(false)).unwrap();
    let mut small = limits.clone();
    small.max_manifest_bytes = 64;
    assert!(generate_nup_mapping(&input, &small, &token).is_err());
    token.cancel();
    assert!(generate_nup_mapping(&input, &limits, &token).is_err());
}

#[test]
fn generation_selects_only_unambiguous_valid_control_metadata() {
    let dir = work("generation-control");
    let input = dir.join("source.nup");
    for case in 0..3 {
        let mut control = vec![Node::File(
            "control.nacp",
            if case == 2 {
                vec![0; 8]
            } else {
                nacp(&[(0, "Generated title")])
            },
        )];
        if case == 1 {
            control.push(Node::Dir(
                "other",
                vec![Node::File("control.nacp", nacp(&[(0, "Other edition")]))],
            ));
        }
        let bytes = build_with_extras(
            &[base(AUX), base(APP)],
            true,
            &[(
                1,
                vec![
                    Node::Dir("meta", vec![Node::File("Application.cnmt", cnmt())]),
                    Node::Dir("control", control),
                ],
            )],
        );
        fs::write(&input, bytes).unwrap();
        let generated = generate_nup_mapping(
            &input,
            &ImportLimits::default(),
            &crate::CancellationToken::default(),
        );
        if case == 2 {
            assert!(generated.is_err());
        } else {
            assert_eq!(
                generated
                    .unwrap()
                    .contains("nacp\tpkg-1\tcontrol/control.nacp"),
                case == 0
            );
        }
    }
}
