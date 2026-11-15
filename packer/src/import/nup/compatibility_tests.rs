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
    let error = crate::cli::run(args.clone(), None, None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("does not conform to NUP v1"));
    assert!(error.contains("--allow-nonconforming-nup"));
    assert!(!mapping.exists());
    args.push("--allow-nonconforming-nup".into());
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
fn nonconforming_cli_requires_acknowledgment_even_with_an_existing_mapping() {
    let dir = work("spec-acknowledgment");
    let input = dir.join("source.nup");
    let map = dir.join("existing.nxnupmap");
    let output = dir.join("output.nx");
    let bytes = generation_fixture(false);
    fs::write(&input, &bytes).unwrap();
    let limits = ImportLimits::default();
    let token = crate::CancellationToken::default();
    let preflight = inspect_nup_spec(&input, &limits, &token).unwrap();
    assert!(preflight.violations.iter().any(|v| v.contains("2 BASE")));
    assert!(preflight.violations.iter().any(|v| v.contains("meta")));
    fs::write(&map, generate_nup_mapping(&input, &limits, &token).unwrap()).unwrap();
    let mut pack = vec![
        "pack-nup".into(),
        input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "--nup-map".into(),
        map.to_string_lossy().into_owned(),
        "--platform".into(),
        "switch2".into(),
    ];
    let error = crate::cli::run(pack.clone(), None, None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("does not conform"));
    assert!(!output.exists());
    pack.push("--allow-nonconforming-nup".into());
    crate::cli::run(pack, None, None).unwrap();
    let mut check = vec![
        "check-nup".into(),
        input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "--nup-map".into(),
        map.to_string_lossy().into_owned(),
    ];
    assert!(crate::cli::run(check.clone(), None, None)
        .unwrap_err()
        .to_string()
        .contains("--allow-nonconforming-nup"));
    check.push("--allow-nonconforming-nup".into());
    crate::cli::run(check, None, None).unwrap();
    // Consent cannot override a damaged TOC.
    let mut corrupt = bytes;
    corrupt[0x90] ^= 1;
    fs::write(&input, corrupt).unwrap();
    assert!(inspect_nup_spec(&input, &limits, &token)
        .unwrap_err()
        .to_string()
        .contains("SHA-256"));
    let mapping_output = dir.join("corrupt.nxnupmap");
    assert!(crate::cli::run(
        vec![
            "map-nup".into(),
            input.to_string_lossy().into_owned(),
            mapping_output.to_string_lossy().into_owned(),
            "--assume-complete".into(),
            "--allow-nonconforming-nup".into()
        ],
        None,
        None
    )
    .is_err());
    assert!(!mapping_output.exists());
}

#[test]
fn preflight_distinguishes_canonical_input_and_wrong_numeric_version() {
    let dir = work("preflight-version");
    let input = dir.join("source.nup");
    let limits = ImportLimits::default();
    let token = crate::CancellationToken::default();
    fs::write(&input, build(&[base(APP)], true)).unwrap();
    assert!(inspect_nup_spec(&input, &limits, &token)
        .unwrap()
        .violations
        .is_empty());
    let mut meta = cnmt();
    meta[8..12].copy_from_slice(&5u32.to_le_bytes());
    fs::write(
        &input,
        build_with_extras(
            &[base(APP)],
            true,
            &[(
                0,
                vec![Node::Dir(
                    "meta",
                    vec![Node::File("Application.cnmt", meta)],
                )],
            )],
        ),
    )
    .unwrap();
    let warnings = inspect_nup_spec(&input, &limits, &token)
        .unwrap()
        .violations;
    assert!(warnings
        .iter()
        .any(|v| v.contains("version 0") && v.contains("version 5")));
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

#[test]
fn legacy_dlc_keeps_native_marker_under_matching_newer_application() {
    let dir = work("legacy-dlc");
    let input = dir.join("source.nup");
    let map = dir.join("source.nxnupmap");
    // Legacy DLC shares the exact application, but not the executable platform.
    for case in 0..4 {
        let mut dlc = vec![0u8; 0x30];
        dlc[..8].copy_from_slice(&(APP + 0x1001).to_le_bytes());
        dlc[0xc] = 0x82;
        dlc[0xe] = 0x10;
        dlc[0x20..0x28]
            .copy_from_slice(&(if case == 1 { APP + 0x10000 } else { APP }).to_le_bytes());
        if case == 2 {
            dlc[0xd] = 2;
        }
        if case == 3 {
            dlc.resize(0x68, 0);
            dlc[0x10] = 1;
            dlc[0x30 + 0x36] = 1;
        }
        let bytes = build_with_extras(
            &[base(AUX), base(APP), addon(APP)],
            true,
            &[
                (
                    1,
                    vec![Node::Dir(
                        "meta",
                        vec![Node::File("Application.cnmt", cnmt())],
                    )],
                ),
                (
                    2,
                    vec![Node::Dir(
                        "meta",
                        vec![Node::File("AddOnContent.cnmt", dlc.clone())],
                    )],
                ),
            ],
        );
        fs::write(&input, &bytes).unwrap();
        let result = generate_nup_mapping(
            &input,
            &ImportLimits::default(),
            &crate::CancellationToken::default(),
        );
        if case != 0 {
            assert!(result.is_err(), "case {case}");
            continue;
        }
        fs::write(&map, result.unwrap()).unwrap();
        let output = dir.join("legacy.nx");
        convert_nup(
            &input,
            &output,
            &NupOptions {
                mapping: Some(map.clone()),
                platform: Some(Platform::Switch2),
                stream: true,
                ..Default::default()
            },
            &ImportLimits::default(),
            &WriterOptions::default(),
        )
        .unwrap();
        assert_eq!(
            check_nup_with_mapping(
                &input,
                &output,
                false,
                &map,
                &ImportLimits::default(),
                &WriterOptions::default()
            )
            .unwrap()
            .snapshots,
            3
        );
        let summary = crate::inspect_archive(
            &output,
            &WriterOptions::default(),
            crate::IntegrityPolicy::Strict,
        )
        .unwrap();
        assert_eq!(summary.applications[0].platform, Platform::Switch2);
        assert!(summary.applications[0]
            .snapshots
            .iter()
            .any(|s| s.snapshot_type == SnapshotType::Dlc && s.program_id.is_none()));
        assert_eq!(fs::read(&input).unwrap(), bytes);
    }
}

#[test]
fn automatic_mapping_reports_missing_launch_evidence_without_claiming_missing_files() {
    let dir = work("data-patch-blocker");
    let input = dir.join("source.nup");
    let mut patch = vec![0u8; 0x40];
    patch[..8].copy_from_slice(&(APP + 0x1801).to_le_bytes());
    patch[0xc] = 0x84;
    patch[0xd] = 1;
    patch[0xe] = 0x20;
    patch[0x20..0x28].copy_from_slice(&(APP + 0x1001).to_le_bytes());
    patch[0x28..0x30].copy_from_slice(&APP.to_le_bytes());
    let bytes = build_with_extras(
        &[base(AUX), addon(APP)],
        true,
        &[(
            1,
            vec![Node::Dir("meta", vec![Node::File("DataPatch.cnmt", patch)])],
        )],
    );
    fs::write(&input, bytes).unwrap();
    let error = generate_nup_mapping(
        &input,
        &ImportLimits::default(),
        &crate::CancellationToken::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(!error.contains("contains DataPatch metadata"));
    assert!(error.contains("Automatic launch-role inference is unavailable"));
    assert!(error.contains("does not prove executable data is missing"));
    assert!(error.contains("explicit mapping"));
    assert!(!error.contains("conflicting CNMT applications"));
}

fn data_patch_cnmt(data: u64, patch: u64, version: u32, minimum: u32) -> Vec<u8> {
    let mut b = vec![0u8; 0x78];
    b[..8].copy_from_slice(&patch.to_le_bytes());
    b[8..12].copy_from_slice(&version.to_le_bytes());
    b[0xc] = 0x84;
    b[0xd] = 1;
    b[0xe] = 0x20;
    b[0x10] = 1;
    b[0x40 + 0x36] = 2;
    b[0x40 + 32..0x40 + 48].fill(0x5a);
    b[0x20..0x28].copy_from_slice(&data.to_le_bytes());
    b[0x28..0x30].copy_from_slice(&APP.to_le_bytes());
    b[0x30..0x34].copy_from_slice(&minimum.to_le_bytes());
    b
}

#[test]
fn materialized_datapatch_retains_dlc_target_patch_metadata_and_version() {
    let dir = work("datapatch-materialized");
    let source = dir.join("source.nup");
    let map = dir.join("source.nxnupmap");
    let mut primary = base(APP);
    primary.version = 10;
    let mut auxiliary = base(AUX);
    auxiliary.version = 10;
    let mut program_meta = cnmt();
    program_meta[8..12].copy_from_slice(&10u32.to_le_bytes());
    let mut base_meta = vec![0u8; 0x38];
    base_meta[..8].copy_from_slice(&(APP + 0x1001).to_le_bytes());
    base_meta[0xc] = 0x82;
    base_meta[0xd] = 1;
    base_meta[0xe] = 0x18;
    base_meta[0x20..0x28].copy_from_slice(&APP.to_le_bytes());
    base_meta[0x30..0x38].copy_from_slice(&(APP + 0x1801).to_le_bytes());
    let mut original = addon(APP);
    original.romfs = Some(vec![Node::File("removed.bin", b"old content".to_vec())]);
    let mut patch = addon(APP);
    patch.romfs = Some(vec![Node::File(
        "new.bin",
        b"complete updated view".to_vec(),
    )]);
    let native = data_patch_cnmt(APP + 0x1001, APP + 0x1801, 7, 5);
    let bytes = build_with_extras(
        &[auxiliary, primary, original, patch],
        true,
        &[
            (
                1,
                vec![Node::Dir(
                    "meta",
                    vec![Node::File("Application.cnmt", program_meta)],
                )],
            ),
            (
                2,
                vec![Node::Dir(
                    "meta",
                    vec![Node::File("AddOnContent.cnmt", base_meta)],
                )],
            ),
            (
                3,
                vec![Node::Dir(
                    "meta",
                    vec![Node::File("DataPatch.cnmt", native.clone())],
                )],
            ),
        ],
    );
    fs::write(&source, &bytes).unwrap();
    let limits = ImportLimits::default();
    let token = crate::CancellationToken::default();
    let text = generate_nup_mapping(&source, &limits, &token).unwrap();
    assert!(text.contains("dlc\t7\tcomplete"));
    assert!(text.contains("addon\tpkg-3\t0100000012341001"));
    assert!(
        text.contains("dependency\tpkg-3\trequires\tpkg-0")
            || text.contains("dependency\tpkg-3\trequires\tpkg-1")
    );
    fs::write(&map, &text).unwrap();
    for stream in [false, true] {
        let output = dir.join(format!("{stream}.nx"));
        convert_nup(
            &source,
            &output,
            &NupOptions {
                mapping: Some(map.clone()),
                platform: Some(Platform::Switch2),
                stream,
                ..Default::default()
            },
            &limits,
            &WriterOptions::default(),
        )
        .unwrap();
        assert_eq!(
            check_nup_with_mapping(
                &source,
                &output,
                false,
                &map,
                &limits,
                &WriterOptions::default()
            )
            .unwrap()
            .snapshots,
            4
        );
        let summary = crate::inspect_archive(
            &output,
            &WriterOptions::default(),
            crate::IntegrityPolicy::Strict,
        )
        .unwrap();
        let patched = summary.applications[0]
            .snapshots
            .iter()
            .find(|s| s.snapshot_type == SnapshotType::Dlc && s.numeric_version == 7)
            .unwrap();
        assert_eq!(patched.addon_title_id, Some(APP + 0x1001));
        assert!(patched.program_id.is_none());
        assert_eq!(patched.dependencies.len(), 1);
        let files = crate::inspect_snapshot_files(
            &output,
            patched.identity,
            &WriterOptions::default(),
            crate::IntegrityPolicy::Strict,
        )
        .unwrap();
        assert!(files.iter().any(|f| f.path == "romfs/new.bin"));
        assert!(!files.iter().any(|f| f.path == "romfs/removed.bin"));
        let extracted = dir.join(format!("extracted-{stream}"));
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
        let snapshot = manifest
            .snapshots
            .iter()
            .find(|s| s.numeric_version == 7)
            .unwrap();
        assert_eq!(
            fs::read(
                extracted
                    .join(&snapshot.root)
                    .join("data/__nup_root__/meta/DataPatch.cnmt")
            )
            .unwrap(),
            native
        );
    }
    assert_eq!(fs::read(&source).unwrap(), bytes);
}

#[test]
fn datapatch_mapping_rejects_bad_target_minimum_version_and_forward_link() {
    let dir = work("datapatch-reject");
    let source = dir.join("source.nup");
    for case in 0..5 {
        let mut native = data_patch_cnmt(
            if case == 0 {
                APP + 0x1002
            } else {
                APP + 0x1001
            },
            APP + 0x1801,
            if case == 4 { 0 } else { 7 },
            if case == 1 { 5 } else { 0 },
        );
        if case == 2 {
            native.truncate(0x30);
        }
        let mut base_meta = vec![0u8; 0x38];
        base_meta[..8].copy_from_slice(&(APP + 0x1001).to_le_bytes());
        base_meta[0xc] = 0x82;
        base_meta[0xd] = 1;
        base_meta[0xe] = 0x18;
        base_meta[0x20..0x28].copy_from_slice(&APP.to_le_bytes());
        base_meta[0x30..0x38].copy_from_slice(
            &(if case == 3 {
                APP + 0x1802
            } else {
                APP + 0x1801
            })
            .to_le_bytes(),
        );
        let bytes = build_with_extras(
            &[base(APP), base(AUX), addon(APP), addon(APP)],
            true,
            &[
                (
                    0,
                    vec![Node::Dir(
                        "meta",
                        vec![Node::File("Application.cnmt", cnmt())],
                    )],
                ),
                (
                    2,
                    vec![Node::Dir(
                        "meta",
                        vec![Node::File("AddOnContent.cnmt", base_meta)],
                    )],
                ),
                (
                    3,
                    vec![Node::Dir(
                        "meta",
                        vec![Node::File("DataPatch.cnmt", native)],
                    )],
                ),
            ],
        );
        fs::write(&source, bytes).unwrap();
        assert!(
            generate_nup_mapping(
                &source,
                &ImportLimits::default(),
                &crate::CancellationToken::default()
            )
            .is_err(),
            "case {case}"
        );
    }
}

#[test]
fn explicit_nonzero_launch_program_is_primary_without_rewriting_its_id() {
    let dir = work("nonzero-launch");
    let source = dir.join("source.nup");
    let map = dir.join("source.nxnupmap");
    let output = dir.join("source.nx");
    let mut legacy = vec![0u8; 0x30];
    legacy[..8].copy_from_slice(&(APP + 0x1001).to_le_bytes());
    legacy[0xc] = 0x82;
    legacy[0xe] = 0x10;
    legacy[0x20..0x28].copy_from_slice(&APP.to_le_bytes());
    let bytes = build_with_extras(
        &[base(AUX), addon(APP), addon(APP)],
        true,
        &[
            (
                1,
                vec![Node::Dir(
                    "meta",
                    vec![Node::File("AddOnContent.cnmt", legacy)],
                )],
            ),
            (
                2,
                vec![Node::Dir(
                    "meta",
                    vec![Node::File(
                        "DataPatch.cnmt",
                        data_patch_cnmt(APP + 0x1001, APP + 0x1801, 7, 0),
                    )],
                )],
            ),
        ],
    );
    fs::write(&source, &bytes).unwrap();
    let hash = bytes[24..56]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let text=format!("NXNUPMAP\t1\t{hash}\nNXIMPORT\t1\nsnapshot\tlaunch\tpackage-0\tswitch2\t{APP:016x}\t{AUX:016x}\tprimary\tbase\t0\tcomplete\tmaterialized-v1\tmeta-acid3072-v1\nsnapshot\told\tpackage-1\tswitch2\t{APP:016x}\t-\tnone\tdlc\t0\tcomplete\tmaterialized-v1\tnone\naddon\told\t{:016x}\nsnapshot\tpatched\tpackage-2\tswitch2\t{APP:016x}\t-\tnone\tdlc\t7\tcomplete\tmaterialized-v1\tnone\naddon\tpatched\t{:016x}\n",APP+0x1001,APP+0x1001);
    fs::write(&map, text).unwrap();
    let limits = ImportLimits::default();
    inspect_nup_with_mapping(&source, &map, &limits, &crate::CancellationToken::default()).unwrap();
    convert_nup(
        &source,
        &output,
        &NupOptions {
            mapping: Some(map.clone()),
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
    assert_eq!(summary.applications[0].programs.len(), 1);
    assert_eq!(summary.applications[0].programs[0].program_id, AUX);
    assert_eq!(
        summary.applications[0].programs[0].kind,
        ProgramKind::Primary
    );
    assert_eq!(
        check_nup_with_mapping(
            &source,
            &output,
            false,
            &map,
            &limits,
            &WriterOptions::default()
        )
        .unwrap()
        .snapshots,
        3
    );
    assert_eq!(fs::read(source).unwrap(), bytes);
}

fn selected_launch_fixture(known_version: Option<u32>, defect: u8) -> Vec<u8> {
    let mut selected = nacp(&[(0, "Selected edition")]);
    selected[0x3070..0x3078].copy_from_slice(&(APP + 0x1000).to_le_bytes());
    selected[0x3212] = 1;
    selected[0x3216] = 1;
    selected[0x3217] = 1;
    selected[0x34d1] = 1;
    selected[0x34d4..0x34d8].copy_from_slice(&2u32.to_le_bytes());
    match defect {
        1 => selected[0x34d4] = 0,
        2 => selected[0x34d1] = 2,
        3 => selected[0x3212] = 0,
        4 => selected[0x3070..0x3078].copy_from_slice(&(APP + 0x2000).to_le_bytes()),
        _ => {}
    }
    let mut original = selected.clone();
    if defect == 5 {
        original[0x3060] = b'9';
    }
    let mut root = selected.clone();
    root[0x3216] = 0;
    root[0x3217] = 0;
    root[0x34d1] = 0;
    root[0x34d4] = 1;
    let mut program = base(AUX);
    program.meta = if defect == 6 {
        vec![]
    } else {
        vec![(2, selected)]
    };
    let mut program_extras = vec![Node::Dir(
        "control",
        vec![
            Node::File("control.nacp", root),
            Node::Dir("Acd_1", vec![Node::File("control.nacp", original)]),
        ],
    )];
    if let Some(version) = known_version {
        let mut meta = vec![0u8; 0x30 + 0x38];
        meta[..8].copy_from_slice(&APP.to_le_bytes());
        meta[8..12].copy_from_slice(&version.to_le_bytes());
        meta[0xc] = 0x80;
        meta[0xd] = 1;
        meta[0xe] = 0x10;
        meta[0x10] = 1;
        meta[0x20..0x28].copy_from_slice(&(APP + 0x800).to_le_bytes());
        meta[0x30 + 0x36] = 1;
        meta[0x30 + 0x37] = 1;
        program_extras.push(Node::Dir(
            "meta",
            vec![Node::File("Application.cnmt", meta)],
        ));
    }
    build_with_extras(
        &[program, addon(APP)],
        true,
        &[
            (0, program_extras),
            (
                1,
                vec![Node::Dir(
                    "meta",
                    vec![Node::File(
                        "DataPatch.cnmt",
                        data_patch_cnmt(APP + 0x1001, APP + 0x1801, 7, 5),
                    )],
                )],
            ),
        ],
    )
}

#[test]
fn selected_nacp_launch_preserves_unknown_versions_and_rejects_known_conflicts() {
    let dir = work("selected-nacp-version");
    let source = dir.join("source.nup");
    let map = dir.join("source.nxnupmap");
    let limits = ImportLimits {
        compare_source_bytes: true,
        ..ImportLimits::default()
    };
    let token = crate::CancellationToken::default();
    for known in [None, Some(0), Some(2), Some(10)] {
        let bytes = selected_launch_fixture(known, 0);
        fs::write(&source, &bytes).unwrap();
        let preflight = inspect_nup_spec(&source, &limits, &token).unwrap();
        assert_eq!(
            !preflight.unresolved_requirements.is_empty(),
            known.is_none()
        );
        let generated = generate_nup_mapping(&source, &limits, &token);
        if matches!(known, Some(0 | 2)) {
            assert!(generated
                .unwrap_err()
                .to_string()
                .contains("authoritative executable CNMT"));
            continue;
        }
        let text = generated.unwrap();
        assert!(text.contains("nacp\tpkg-0\tcontrol/Acd_1/control.nacp"));
        assert!(text.contains(&format!(
            "{AUX:016x}\tprimary\tbase\t{}\tcomplete",
            known.unwrap_or(0)
        )));
        assert_eq!(
            text.contains("dependency\tpkg-1\trequires\tpkg-0"),
            known.is_some()
        );
        fs::write(&map, &text).unwrap();
        let output = dir.join(format!("{known:?}.nx"));
        convert_nup(
            &source,
            &output,
            &NupOptions {
                mapping: Some(map.clone()),
                platform: Some(Platform::Switch2),
                stream: true,
                ..Default::default()
            },
            &limits,
            &WriterOptions::default(),
        )
        .unwrap();
        crate::inspect_archive(
            &output,
            &WriterOptions::default(),
            crate::IntegrityPolicy::Strict,
        )
        .unwrap();
        check_nup_with_mapping(
            &source,
            &output,
            false,
            &map,
            &limits,
            &WriterOptions::default(),
        )
        .unwrap();
        let mut f = File::open(&source).unwrap();
        let (toc, _) = read_toc(&mut f, &limits).unwrap();
        let parsed = Mapping::read(&map, &source, &toc, &limits, &token).unwrap();
        let provenance = parsed.provenance(&toc);
        assert_eq!(
            provenance.contains("unresolved-requirement\t1\t"),
            known.is_none()
        );
        if known.is_none() {
            assert!(provenance.contains("\t5\tunknown-authoritative-application-version"));
            assert_eq!(parsed.unresolved_requirements.len(), 1);
            let forged = text.replace("primary\tbase\t0\t", "primary\tbase\t10\t");
            fs::write(&map, forged).unwrap();
            assert!(inspect_nup_with_mapping(&source, &map, &limits, &token).is_err());
            fs::write(&map, format!("{text}dependency\tpkg-1\trequires\tpkg-0\n")).unwrap();
            assert!(inspect_nup_with_mapping(&source, &map, &limits, &token)
                .err()
                .unwrap()
                .to_string()
                .contains("cannot assert an unknown"));
        }
        assert_eq!(fs::read(&source).unwrap(), bytes);
    }
}

#[test]
fn selected_nacp_launch_rejects_conflicts_missing_access_and_missing_target() {
    let dir = work("selected-nacp-conflict");
    let source = dir.join("source.nup");
    for defect in 1..=6 {
        fs::write(&source, selected_launch_fixture(None, defect)).unwrap();
        assert!(
            generate_nup_mapping(
                &source,
                &ImportLimits::default(),
                &crate::CancellationToken::default()
            )
            .is_err(),
            "defect {defect}"
        );
    }
}

#[test]
fn unresolved_requirements_need_fresh_cli_opt_in_even_for_an_existing_map() {
    let dir = work("unresolved-requirement-consent");
    let source = dir.join("source.nup");
    let map = dir.join("source.nxnupmap");
    let output = dir.join("out.nx");
    fs::write(&source, selected_launch_fixture(None, 0)).unwrap();
    let text = generate_nup_mapping(
        &source,
        &ImportLimits::default(),
        &crate::CancellationToken::default(),
    )
    .unwrap();
    fs::write(&map, text).unwrap();
    let mut args = vec![
        "pack-nup".into(),
        source.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "--platform".into(),
        "switch2".into(),
        "--nup-map".into(),
        map.to_string_lossy().into_owned(),
        "--allow-nonconforming-nup".into(),
    ];
    let error = crate::cli::run(args.clone(), None, None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("--allow-unresolved-nup-requirements"));
    assert!(!output.exists());
    args.push("--allow-unresolved-nup-requirements".into());
    crate::cli::run(args, None, None).unwrap();
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
    let mut retained = false;
    fn visit(path: &Path, retained: &mut bool) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, retained);
            } else if path.file_name().unwrap() == "__nup_mapping__.txt" {
                let text = fs::read_to_string(path).unwrap();
                assert!(text.contains("unresolved-requirement\t1\t"));
                *retained = true;
            }
        }
    }
    visit(&extracted, &mut retained);
    assert!(retained);
}
