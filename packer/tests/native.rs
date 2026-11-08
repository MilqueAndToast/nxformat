//! Native inventory tests on synthetic, redistributable DNSP/DXCI fixtures.

use nxformat_packer::native::synthetic::{dnsp, dxci, title_entries, TitleSpec};
use nxformat_packer::native::{
    inspect_source, inventory, ContentMetaType, Inventory, InventoryMode, InventoryOptions,
    IssueCode, NativeGeneration, Severity, SourceFormat, SourceStatus, VerificationState,
};
use nxformat_packer::{
    inspect_archive, repack_archive, write_archive, ArchiveInput, IntegrityPolicy, Platform,
    ProgramKind, RepackPlan, SnapshotType, WriterOptions,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("nx-native-test-{}-{id}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.0.join(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&p, bytes).unwrap();
        p
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const APP: u64 = 0x0100_0000_0001_0000;

fn fast() -> InventoryOptions {
    InventoryOptions::default()
}

fn strict() -> InventoryOptions {
    InventoryOptions {
        mode: InventoryMode::Strict,
        ..InventoryOptions::default()
    }
}

fn codes(inv: &Inventory) -> Vec<(Severity, IssueCode)> {
    inv.all_issues().map(|i| (i.severity, i.code)).collect()
}

fn has(inv: &Inventory, severity: Severity, code: IssueCode) -> bool {
    codes(inv).contains(&(severity, code))
}

fn run(dir: &Path, options: &InventoryOptions) -> Inventory {
    inventory(&[dir.to_owned()], options).unwrap()
}

#[test]
fn base_update_dlc_are_associated_from_native_records_not_names() {
    let temp = TempDir::new();
    // Deliberately misleading file and directory names.
    temp.write(
        "dlc/update [v999].dnsp",
        &dnsp(&title_entries(&TitleSpec::application(APP, 0, 1))),
    );
    temp.write(
        "base/dlc.dnsp",
        &dnsp(&title_entries(&TitleSpec::patch(APP, 0x20000, 2))),
    );
    temp.write(
        "updates/base.xci",
        &dnsp(&title_entries(&TitleSpec::add_on(APP, APP + 0x1001, 3, 3))),
    );
    temp.write("notes.txt", b"not a container");
    for options in [fast(), strict()] {
        let inv = run(temp.path(), &options);
        assert_eq!(inv.sources.len(), 3, "text file is ignored");
        assert_eq!(inv.stats.files_ignored, 1);
        assert!(inv
            .sources
            .iter()
            .all(|s| s.format == SourceFormat::Dnsp && s.status == SourceStatus::Supported));
        assert_eq!(inv.error_count(), 0, "{:?}", codes(&inv));
        assert_eq!(inv.applications.len(), 1);
        let app = &inv.applications[0];
        assert_eq!(app.platform, Platform::Switch1);
        assert_eq!(app.application_id, APP);
        assert_eq!(app.bases.len(), 1);
        assert_eq!(app.updates.len(), 1);
        assert_eq!(app.dlc.len(), 1);
        assert_eq!(app.updates[0].version, 0x20000);

        let base = inv
            .sources
            .iter()
            .flat_map(|s| &s.metas)
            .find(|m| m.meta.meta_type == ContentMetaType::Application)
            .unwrap();
        assert_eq!(base.meta.generation, NativeGeneration::Switch1);
        assert_eq!(base.meta.build_id, Some([1; 32]));
        let control = base.meta.control.as_ref().unwrap();
        assert_eq!(control.display_version.as_deref(), Some("1.0"));
        assert_eq!(control.titles[0].language, "AmericanEnglish");
        assert_eq!(control.titles[0].name.as_deref(), Some("Synthetic Title"));
        assert_eq!(control.icons, vec![("AmericanEnglish".to_string(), 5)]);
        let expected = if options.mode == InventoryMode::Strict {
            VerificationState::Verified
        } else {
            VerificationState::NotChecked
        };
        assert_eq!(base.meta.verification, expected);

        let plan = inv.plan();
        assert!(plan.blocked.is_empty(), "{:?}", plan.blocked);
        let planned = &plan.applications[0];
        assert_eq!(planned.programs.len(), 1);
        let by_type = |t| {
            planned
                .snapshots
                .iter()
                .find(|s| s.snapshot_type == t)
                .unwrap()
        };
        let b = by_type(SnapshotType::Base);
        let u = by_type(SnapshotType::Update);
        let d = by_type(SnapshotType::Dlc);
        assert_eq!(b.files.len(), 3, "meta + program + control");
        assert_eq!(
            u.files.len(),
            6,
            "update units plus the base units it patches"
        );
        assert_eq!(d.files.len(), 2);
        assert_eq!(b.program_id, Some(APP));
        assert_eq!(u.program_id, Some(APP));
        assert_eq!(d.program_id, None, "DLC never gets an invented Program ID");
        assert_eq!(u.requires.as_ref().unwrap().numeric_version, 0);
        assert!(b.content_id.is_some());
    }
}

#[test]
fn dxci_secure_partition_holds_base_and_update() {
    let temp = TempDir::new();
    let mut secure = title_entries(&TitleSpec::application(APP, 0, 1));
    secure.extend(title_entries(&TitleSpec::patch(APP, 0x10000, 2)));
    secure.sort_by(|a, b| a.0.cmp(&b.0));
    let image = dxci(&[
        ("update", vec![("sysupdate.bin".into(), vec![0; 64])]),
        ("normal", vec![]),
        ("secure", secure),
    ]);
    temp.write("game.bin", &image);
    let inv = run(temp.path(), &strict());
    assert_eq!(inv.sources[0].format, SourceFormat::Dxci);
    assert_eq!(inv.sources[0].metas.len(), 2);
    assert!(inv.sources[0]
        .units
        .iter()
        .all(|u| u.partition.as_deref() == Some("secure")));
    assert_eq!(inv.error_count(), 0, "{:?}", codes(&inv));
    let plan = inv.plan();
    assert_eq!(plan.applications[0].snapshots.len(), 2);
}

#[test]
fn duplicate_content_name_across_dxci_partitions_blocks_planning_unless_identical() {
    let secure = title_entries(&TitleSpec::application(APP, 0, 1));
    let duplicate = secure
        .iter()
        .find(|(name, _)| name.ends_with(".nca") && !name.ends_with(".cnmt.nca"))
        .unwrap()
        .clone();
    // A byte-identical copy (early game cards) is the same content.
    let temp = TempDir::new();
    temp.write(
        "copy.xci",
        &dxci(&[
            ("secure", secure.clone()),
            ("normal", vec![duplicate.clone()]),
        ]),
    );
    let inv = run(temp.path(), &fast());
    assert!(!has(&inv, Severity::Error, IssueCode::ContentMismatch));
    assert_eq!(inv.plan().applications.len(), 1);
    // A copy with other bytes under the same name stays ambiguous.
    let mut changed = duplicate;
    let last = changed.1.len() - 1;
    changed.1[last] ^= 1;
    let temp = TempDir::new();
    temp.write(
        "overlap.xci",
        &dxci(&[("secure", secure), ("normal", vec![changed])]),
    );
    let inv = run(temp.path(), &fast());
    assert!(has(&inv, Severity::Error, IssueCode::ContentMismatch));
    assert!(inv.plan().applications.is_empty());
}

#[test]
fn update_without_base_is_blocked_even_without_bktr() {
    let temp = TempDir::new();
    let mut patch = TitleSpec::patch(APP, 0x10000, 2);
    patch.program.as_mut().unwrap().bktr = false;
    temp.write("orphan.nsp", &dnsp(&title_entries(&patch)));
    let inv = run(temp.path(), &fast());
    assert!(has(&inv, Severity::Error, IssueCode::MissingBase));
    let plan = inv.plan();
    assert!(plan.applications.is_empty());
    assert_eq!(plan.blocked.len(), 1);
}

#[test]
fn overlapping_nca_sections_are_rejected() {
    let temp = TempDir::new();
    let mut entries = title_entries(&TitleSpec::application(APP, 0, 1));
    let program = entries.iter_mut().find(|(_, b)| b[0x205] == 0).unwrap();
    let first_start = program.1[0x240..0x244].to_vec();
    program.1[0x250..0x254].copy_from_slice(&first_start);
    temp.write("overlap.nsp", &dnsp(&entries));
    let inv = run(temp.path(), &fast());
    assert!(has(&inv, Severity::Error, IssueCode::InvalidStructure));
    assert!(inv.plan().applications.is_empty());
}

#[test]
fn program_mapping_only_emits_programs_owned_by_selected_snapshots() {
    let temp = TempDir::new();
    temp.write(
        "base.nsp",
        &dnsp(&title_entries(&TitleSpec::application(APP, 0, 1))),
    );
    let mut inv = run(temp.path(), &fast());
    let mut app = inv.plan().applications.remove(0);
    app.programs.push((APP + 0x10, ProgramKind::Auxiliary));
    let selected = app.to_application_input(&|_| true, &|_| None).unwrap();
    assert_eq!(selected.programs.len(), 1);
    assert_eq!(selected.programs[0].program_id, APP);

    inv.sources[0].metas[0].meta.program_ids.push(APP + 0x10);
    let plan = inv.plan();
    assert!(plan.applications.is_empty());
    assert_eq!(plan.blocked.len(), 1);
    assert!(plan.blocked[0]
        .reasons
        .iter()
        .any(|reason| reason.contains("Program ID inventory")));
}

#[test]
fn not_decrypted_encrypted_and_compressed_inputs_are_rejected() {
    let temp = TempDir::new();
    let mut nca3 = title_entries(&TitleSpec::application(APP, 0, 1));
    for (_, bytes) in &mut nca3 {
        bytes[0x200..0x204].copy_from_slice(b"NCA3");
    }
    let a = temp.write("nca3.nsp", &dnsp(&nca3));

    let mut spec = TitleSpec::application(APP, 0, 2);
    spec.encrypted_sections = true;
    let b = temp.write("enc.nsp", &dnsp(&title_entries(&spec)));

    let mut garbage = title_entries(&TitleSpec::application(APP, 0, 3));
    for (_, bytes) in &mut garbage {
        for (i, v) in bytes[..0xC00].iter_mut().enumerate() {
            *v = (i as u8).wrapping_mul(31) ^ 0x5A;
        }
    }
    let c = temp.write("garbage.nsp", &dnsp(&garbage));

    let mut xci = dxci(&[("secure", title_entries(&TitleSpec::application(APP, 0, 4)))]);
    xci[0x100..0x104].copy_from_slice(b"HEAD");
    let d = temp.write("plain.xci", &xci);

    let mut ncz = title_entries(&TitleSpec::application(APP, 0, 5));
    let (name, bytes) = ncz.iter_mut().find(|e| e.1.len() > 0x5000).unwrap();
    bytes[0x4000..0x4008].copy_from_slice(b"NCZSECTN");
    *name = name.replace(".nca", ".ncz");
    let e = temp.write("pack.nsz", &dnsp(&ncz));

    let expect = [
        (a, SourceFormat::EncryptedPackage, IssueCode::NotDecrypted),
        (
            b,
            SourceFormat::EncryptedPackage,
            IssueCode::EncryptedSection,
        ),
        (c, SourceFormat::EncryptedPackage, IssueCode::NotDecrypted),
        (d, SourceFormat::EncryptedPackage, IssueCode::NotDecrypted),
        (
            e,
            SourceFormat::CompressedPackage,
            IssueCode::CompressedUnsupported,
        ),
    ];
    for (path, format, code) in expect {
        let r = inspect_source(&path, &fast());
        assert_eq!(r.format, format, "{}", path.display());
        assert_ne!(r.status, SourceStatus::Supported);
        assert!(r.metas.is_empty());
        assert!(r.issues.iter().any(|i| i.code == code), "{:?}", r.issues);
    }
    assert!(run(temp.path(), &fast()).plan().applications.is_empty());
}

#[test]
fn missing_and_mismatched_content_blocks_planning() {
    let temp = TempDir::new();
    let entries = title_entries(&TitleSpec::application(APP, 0, 1));
    // Drop the Control unit (the only NCA with content type 2).
    let without_control: Vec<_> = entries
        .iter()
        .filter(|(_, b)| b[0x205] != 2)
        .cloned()
        .collect();
    assert_eq!(without_control.len(), entries.len() - 1);
    temp.write("a/base.nsp", &dnsp(&without_control));
    let inv = run(&temp.path().join("a"), &fast());
    assert!(has(&inv, Severity::Error, IssueCode::MissingContent));
    let plan = inv.plan();
    assert!(plan.applications.is_empty());
    assert_eq!(plan.blocked.len(), 1);

    // Swap the content type of the Control unit to Data.
    let mut mismatched = entries.clone();
    let control = mismatched.iter_mut().find(|(_, b)| b[0x205] == 2).unwrap();
    control.1[0x205] = 4;
    temp.write("b/base.nsp", &dnsp(&mismatched));
    let inv = run(&temp.path().join("b"), &fast());
    assert!(has(&inv, Severity::Error, IssueCode::ContentMismatch));
    assert!(inv.plan().applications.is_empty());

    // An extra NCA nobody references is reported but does not block.
    let mut extra = entries.clone();
    let mut stray = title_entries(&TitleSpec::add_on(APP, APP + 0x1001, 0, 9))
        .into_iter()
        .find(|e| !e.0.contains("cnmt"))
        .unwrap();
    stray.0 = format!("{}.nca", "ab".repeat(16));
    extra.push(stray);
    temp.write("c/base.nsp", &dnsp(&extra));
    let inv = run(&temp.path().join("c"), &fast());
    assert!(has(&inv, Severity::Warning, IssueCode::UnreferencedContent));
    assert_eq!(inv.plan().applications.len(), 1);
}

#[test]
fn duplicates_ambiguous_versions_and_missing_relationships() {
    let temp = TempDir::new();
    let base = dnsp(&title_entries(&TitleSpec::application(APP, 0, 1)));
    temp.write("dup/one.nsp", &base);
    temp.write("dup/two.nsp", &base);
    let inv = run(&temp.path().join("dup"), &fast());
    assert!(has(&inv, Severity::Warning, IssueCode::DuplicateContent));
    let plan = inv.plan();
    assert_eq!(
        plan.applications[0].snapshots.len(),
        1,
        "identical copy planned once"
    );

    // Fast mode does not hash all payloads: identical declarations are not
    // sufficient when the actual DNCA bytes differ.
    let mut changed = base.clone();
    *changed.last_mut().unwrap() ^= 1;
    temp.write("changed/one.nsp", &base);
    temp.write("changed/two.nsp", &changed);
    let inv = run(&temp.path().join("changed"), &fast());
    assert!(has(&inv, Severity::Error, IssueCode::AmbiguousVersion));
    assert!(inv.plan().applications.is_empty());

    // Same title/version, different contents.
    temp.write("amb/one.nsp", &base);
    temp.write(
        "amb/two.nsp",
        &dnsp(&title_entries(&TitleSpec::application(APP, 0, 7))),
    );
    let inv = run(&temp.path().join("amb"), &fast());
    assert!(has(&inv, Severity::Error, IssueCode::AmbiguousVersion));
    assert!(inv.plan().applications.is_empty());

    // Update without base, DLC without base.
    temp.write(
        "orphans/u.nsp",
        &dnsp(&title_entries(&TitleSpec::patch(APP, 0x10000, 2))),
    );
    temp.write(
        "orphans/d.nsp",
        &dnsp(&title_entries(&TitleSpec::add_on(APP, APP + 0x1001, 0, 3))),
    );
    let inv = run(&temp.path().join("orphans"), &fast());
    assert!(has(&inv, Severity::Error, IssueCode::MissingBase));
    assert!(has(&inv, Severity::Warning, IssueCode::MissingApplication));
    let plan = inv.plan();
    assert_eq!(plan.blocked.len(), 1);
    assert_eq!(plan.blocked[0].meta_type, ContentMetaType::Patch);
    assert_eq!(
        plan.applications[0].snapshots.len(),
        1,
        "DLC-only is allowed"
    );
    assert_eq!(
        plan.applications[0].snapshots[0].snapshot_type,
        SnapshotType::Dlc
    );

    // Two base versions make the update's required base ambiguous.
    temp.write("multi/b0.nsp", &base);
    temp.write(
        "multi/b1.nsp",
        &dnsp(&title_entries(&TitleSpec::application(APP, 0x10000, 4))),
    );
    temp.write(
        "multi/u.nsp",
        &dnsp(&title_entries(&TitleSpec::patch(APP, 0x20000, 2))),
    );
    let inv = run(&temp.path().join("multi"), &fast());
    assert!(has(&inv, Severity::Warning, IssueCode::AmbiguousVersion));
    let plan = inv.plan();
    assert_eq!(plan.applications[0].snapshots.len(), 2, "both bases");
    assert_eq!(plan.blocked.len(), 1);
    assert!(plan.blocked[0].reasons[0].contains("ambiguous"));
}

#[test]
fn duplicate_cnmt_fields_are_compared_before_payload_io() {
    use nxformat_packer::native::{associate, ContentRecordType};
    let temp = TempDir::new();
    let path = temp.write(
        "base.nsp",
        &dnsp(&title_entries(&TitleSpec::application(APP, 0, 1))),
    );
    let report = inspect_source(&path, &fast());
    // Removing the source makes unnecessary byte comparison observable as IO.
    fs::remove_file(&path).unwrap();
    for field in 0..10 {
        let mut other = report.clone();
        let m = &mut other.metas[0].meta;
        match field {
            0 => m.contents[0].hash[0] ^= 1,
            1 => m.contents[0].record_type = ContentRecordType::Data,
            2 => m.contents[0].id_offset ^= 1,
            3 => m.contents[0].content_id[0] ^= 1,
            4 => m.contents[0].size += 1,
            5 => m.meta_content_id = Some([99; 16]),
            6 => m.patch_id = Some(APP + 0x900),
            7 => m.required_application_version = Some(42),
            8 => m.required_system_version = Some(42),
            _ => m.application_id = APP + 0x2000,
        }
        let inv = associate(vec![report.clone(), other]);
        assert!(
            has(&inv, Severity::Error, IssueCode::AmbiguousVersion),
            "field {field}"
        );
        assert!(!has(&inv, Severity::Error, IssueCode::Io), "field {field}");
        assert!(inv.plan().applications.is_empty(), "field {field}");
    }
    // An equivalent earlier candidate must not trigger IO before a later
    // declaration mismatch is found.
    let mut different = report.clone();
    different.metas[0].meta.contents[0].hash[0] ^= 1;
    let inv = associate(vec![report.clone(), report.clone(), different]);
    assert!(has(&inv, Severity::Error, IssueCode::AmbiguousVersion));
    assert!(!has(&inv, Severity::Error, IssueCode::Io));
    // Ordinary association must not open any payload files.
    let inv = associate(vec![report]);
    assert_eq!(inv.error_count(), 0);
    assert_eq!(inv.plan().applications[0].snapshots.len(), 1);
}

#[test]
fn duplicate_read_errors_block_every_copy_and_are_not_mismatches() {
    use nxformat_packer::native::associate;
    let temp = TempDir::new();
    let bytes = dnsp(&title_entries(&TitleSpec::application(APP, 0, 1)));
    let left = temp.write("a.nsp", &bytes);
    let right = temp.write("b.nsp", &bytes);
    let reports = vec![
        inspect_source(&left, &fast()),
        inspect_source(&right, &fast()),
    ];
    for missing in [false, true] {
        if missing {
            fs::remove_file(&right).unwrap();
        } else {
            fs::write(&right, &bytes[..bytes.len() - 1]).unwrap();
        }
        let inv = associate(reports.clone());
        assert!(has(&inv, Severity::Error, IssueCode::Io));
        assert!(!has(&inv, Severity::Error, IssueCode::AmbiguousVersion));
        assert_eq!(inv.issues[0].source, Some(1));
        assert!(inv.plan().applications.is_empty());
        assert_eq!(inv.plan().blocked.len(), 2);
    }
}

#[test]
fn duplicate_streaming_compares_meta_and_payload_across_chunks_and_containers() {
    use nxformat_packer::native::associate;
    let temp = TempDir::new();
    let mut spec = TitleSpec::application(APP, 0, 1);
    spec.data = Some(vec![0x5a; 3 * 65536 + 17]);
    let entries = title_entries(&spec);
    let left = temp.write("a.nsp", &dnsp(&entries));
    let right = temp.write("b.xci", &dxci(&[("secure", entries.clone())]));
    for options in [fast(), strict()] {
        let a = inspect_source(&left, &options);
        let b = inspect_source(&right, &options);
        let inv = associate(vec![a.clone(), b.clone(), a.clone()]);
        assert_eq!(inv.error_count(), 0);
        assert!(has(&inv, Severity::Warning, IssueCode::DuplicateContent));
        assert_eq!(inv.plan().applications[0].snapshots.len(), 1);
        // Change bytes after inspection to exercise all unit ranges, including
        // Meta bytes and a mismatch beyond the first 64 KiB payload chunk.
        let original = fs::read(&right).unwrap();
        for &index in &b.metas[0].units {
            let unit = &b.units[index];
            let mut changed = original.clone();
            changed[(unit.offset + unit.size - 1) as usize] ^= 1;
            fs::write(&right, changed).unwrap();
            let inv = associate(vec![a.clone(), b.clone()]);
            assert!(has(&inv, Severity::Error, IssueCode::AmbiguousVersion));
            assert!(!has(&inv, Severity::Error, IssueCode::Io));
            assert!(inv.plan().applications.is_empty());
            fs::write(&right, &original).unwrap();
        }
    }
}

#[test]
fn identical_duplicate_native_units_remain_convertible() {
    let temp = TempDir::new();
    let entries = title_entries(&TitleSpec::application(APP, 0, 1));
    temp.write("src/a.nsp", &dnsp(&entries));
    temp.write("src/b.xci", &dxci(&[("secure", entries)]));
    let inv = run(&temp.path().join("src"), &strict());
    let out = temp.path().join("game.nx");
    let options = WriterOptions::default();
    nxformat_packer::conversion::convert_inventory(
        &inv,
        &out,
        &strict(),
        IntegrityPolicy::Strict,
        &options,
    )
    .unwrap();
    let summary = inspect_archive(&out, &options, IntegrityPolicy::Strict).unwrap();
    assert_eq!(summary.applications[0].snapshots.len(), 1);
    assert_eq!(
        summary.applications[0].snapshots[0].snapshot_type,
        SnapshotType::Base
    );
}

#[test]
fn duplicate_payload_mismatch_prevents_conversion_publication() {
    let temp = TempDir::new();
    let bytes = dnsp(&title_entries(&TitleSpec::application(APP, 0, 1)));
    temp.write("src/a.nsp", &bytes);
    let mut changed = bytes.clone();
    *changed.last_mut().unwrap() ^= 1;
    temp.write("src/b.nsp", &changed);
    let inv = run(&temp.path().join("src"), &fast());
    let out = temp.write("old.nx", b"previous destination");
    assert!(nxformat_packer::conversion::convert_inventory(
        &inv,
        &out,
        &fast(),
        IntegrityPolicy::Strict,
        &WriterOptions::default()
    )
    .is_err());
    assert_eq!(fs::read(&out).unwrap(), b"previous destination");
    assert_eq!(fs::read(inv.sources[0].path.clone()).unwrap(), bytes);
    assert_eq!(fs::read(inv.sources[1].path.clone()).unwrap(), changed);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
}

#[test]
fn switch2_and_unknown_generation_markers_are_reported_not_planned() {
    let temp = TempDir::new();
    let mut s2 = TitleSpec::application(APP, 0, 1);
    s2.platform = 1;
    temp.write("s2.nsp", &dnsp(&title_entries(&s2)));
    let mut unknown = TitleSpec::application(APP + 0x2000, 0, 2);
    unknown.platform = 7;
    temp.write("x.nsp", &dnsp(&title_entries(&unknown)));
    let inv = run(temp.path(), &fast());
    assert!(has(&inv, Severity::Error, IssueCode::Switch2Unsupported));
    assert!(has(&inv, Severity::Error, IssueCode::UnknownGeneration));
    assert!(inv
        .applications
        .iter()
        .any(|a| a.platform == Platform::Switch2));
    let plan = inv.plan();
    assert!(plan.applications.is_empty());
    assert_eq!(plan.blocked.len(), 2);
}

fn npdm(program_id: u64) -> Vec<u8> {
    let mut b = vec![0u8; 0xC0];
    b[..4].copy_from_slice(b"META");
    b[0x0C] = 0x37;
    b[0x20..0x2B].copy_from_slice(b"Application");
    b[0x70..0x74].copy_from_slice(&0x80u32.to_le_bytes());
    b[0x74..0x78].copy_from_slice(&0x40u32.to_le_bytes());
    b[0x80..0x84].copy_from_slice(b"ACI0");
    b[0x90..0x98].copy_from_slice(&program_id.to_le_bytes());
    b
}

#[test]
fn extracted_trees_and_standalone_ncas_are_unsupported_with_native_facts() {
    let temp = TempDir::new();
    temp.write("Game/base/exefs/main.npdm", &npdm(APP));
    temp.write("Game/base/romfs/data.bin", b"x");
    temp.write("Game/updates/196608/exefs/main.npdm", &npdm(APP));
    let mut nso = vec![0u8; 0x100];
    nso[..4].copy_from_slice(b"NSO0");
    nso[0x40..0x60].copy_from_slice(&[0xAB; 32]);
    temp.write("Game/updates/196608/exefs/main", &nso);
    for i in 0..50 {
        temp.write(&format!("Game/dlc/0100/0/romfs/f{i}.bin"), b"payload");
    }
    let lone = title_entries(&TitleSpec::application(APP, 0, 1))
        .into_iter()
        .next()
        .unwrap();
    temp.write("loose/unit.dnca", &lone.1);

    let inv = run(temp.path(), &fast());
    let trees: Vec<_> = inv
        .sources
        .iter()
        .filter(|s| s.format == SourceFormat::ExtractedTree)
        .collect();
    assert_eq!(trees.len(), 3);
    assert_eq!(inv.stats.files_ignored, 0, "RomFS trees are not walked");
    let romfs_only = trees
        .iter()
        .filter(|t| t.tree.as_ref().unwrap().program_id.is_none())
        .count();
    assert_eq!(romfs_only, 1);
    for t in &trees {
        assert_eq!(t.status, SourceStatus::Unsupported);
        assert!(matches!(
            t.tree.as_ref().unwrap().program_id,
            None | Some(APP)
        ));
        assert!(t
            .issues
            .iter()
            .any(|i| i.code == IssueCode::ExtractedTreeUnsupported));
    }
    assert!(trees
        .iter()
        .any(|t| t.tree.as_ref().unwrap().main_build_id == Some([0xAB; 32])));
    assert!(has(
        &inv,
        Severity::Error,
        IssueCode::StandaloneNcaUnsupported
    ));
    assert!(inv.applications.is_empty(), "no version is invented");
    assert!(inv.plan().applications.is_empty());
}

/// Deterministic xorshift for corruption tests.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn truncated_and_mutated_containers_never_panic() {
    let temp = TempDir::new();
    let mut secure = title_entries(&TitleSpec::application(APP, 0, 1));
    secure.extend(title_entries(&TitleSpec::add_on(APP, APP + 0x1001, 0, 2)));
    secure.sort_by(|a, b| a.0.cmp(&b.0));
    let images = [dnsp(&secure), dxci(&[("secure", secure.clone())])];
    for (k, image) in images.iter().enumerate() {
        let path = temp.path().join(format!("t{k}.bin"));
        let mut step = 1usize;
        let mut cut = 0usize;
        while cut < image.len() {
            fs::write(&path, &image[..cut]).unwrap();
            let r = inspect_source(&path, &fast());
            assert_ne!(r.status, SourceStatus::Supported, "truncated at {cut}");
            cut += step;
            step = (step * 2).min(4099);
        }
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 + k as u64);
        for _ in 0..400 {
            let mut m = image.clone();
            for _ in 0..1 + rng.next() % 4 {
                let at = (rng.next() as usize) % m.len();
                m[at] ^= 1 << (rng.next() % 8);
            }
            fs::write(&path, &m).unwrap();
            for options in [fast(), strict()] {
                let r = inspect_source(&path, &options);
                let _ = r.metas.len();
            }
        }
    }
}

#[test]
fn strict_detects_payload_corruption_that_fast_does_not_read() {
    let temp = TempDir::new();
    let mut entries = title_entries(&TitleSpec::application(APP, 0, 1));
    let program = entries.iter_mut().find(|(_, b)| b[0x205] == 0).unwrap();
    // Corrupt `main.npdm` bytes inside the ExeFS PFS0 data: the NSO header and
    // all tables stay intact, only the hash tree notices.
    let pos = program
        .1
        .windows(4)
        .rposition(|w| w == b"META")
        .expect("synthetic main.npdm");
    program.1[pos] ^= 0xFF;
    temp.write("base.nsp", &dnsp(&entries));
    let fast_inv = run(temp.path(), &fast());
    assert_eq!(fast_inv.error_count(), 0);
    assert_eq!(fast_inv.plan().applications.len(), 1);
    let strict_inv = run(temp.path(), &strict());
    assert!(has(
        &strict_inv,
        Severity::Error,
        IssueCode::VerificationFailed
    ));
    assert!(strict_inv.plan().applications.is_empty());

    // Strict skips, and reports, sections above its size limit.
    let limited = InventoryOptions {
        max_strict_section_bytes: 16,
        ..strict()
    };
    let inv = run(temp.path(), &limited);
    assert!(has(&inv, Severity::Warning, IssueCode::VerificationSkipped));
}

#[test]
fn resource_limits_are_enforced() {
    let temp = TempDir::new();
    temp.write(
        "base.nsp",
        &dnsp(&title_entries(&TitleSpec::application(APP, 0, 1))),
    );
    let cases = [
        InventoryOptions {
            max_partition_entries: 2,
            ..fast()
        },
        InventoryOptions {
            max_string_table_bytes: 8,
            ..fast()
        },
        InventoryOptions {
            max_meta_bytes: 16,
            ..fast()
        },
    ];
    for options in cases {
        let inv = run(temp.path(), &options);
        assert!(
            has(&inv, Severity::Error, IssueCode::ResourceLimit),
            "{:?}",
            codes(&inv)
        );
        assert!(inv.plan().applications.is_empty());
    }
    let few = InventoryOptions {
        max_sources: 0,
        ..fast()
    };
    assert!(inventory(&[temp.path().to_owned()], &few).is_err());
}

#[test]
fn large_dlc_payload_is_not_read_by_inventory() {
    let temp = TempDir::new();
    let mut dlc = TitleSpec::add_on(APP, APP + 0x1001, 0, 3);
    dlc.data = Some(vec![0x5A; 64 * 1024 * 1024]);
    let image = dnsp(&title_entries(&dlc));
    let size = image.len() as u64;
    temp.write("big.nsp", &image);
    drop(image);
    for options in [fast(), strict()] {
        let inv = run(temp.path(), &options);
        assert_eq!(inv.error_count(), 0, "{:?}", codes(&inv));
        assert!(
            inv.stats.bytes_read < 256 * 1024,
            "{:?} read {} of {size} bytes",
            options.mode,
            inv.stats.bytes_read
        );
    }
}

#[test]
fn inventory_output_is_deterministic_json() {
    let temp = TempDir::new();
    temp.write(
        "a ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â© name.nsp",
        &dnsp(&title_entries(&TitleSpec::application(APP, 0, 1))),
    );
    temp.write(
        "b.nsp",
        &dnsp(&title_entries(&TitleSpec::patch(APP, 0x10000, 2))),
    );
    let one = run(temp.path(), &fast());
    let two = run(temp.path(), &fast());
    let strip = |j: String| {
        let at = j.find("\"elapsed_ms\":").unwrap();
        let end = at + j[at..].find(',').unwrap();
        format!("{}{}", &j[..at], &j[end..])
    };
    let a = strip(one.to_json());
    assert_eq!(a, strip(two.to_json()));
    assert!(a.contains("a ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â© name.nsp"));
    assert!(a.contains("\"application_id\":\"0100000000010000\""));
    let depth = a.chars().fold(0i32, |d, c| match c {
        '{' | '[' => d + 1,
        '}' | ']' => d - 1,
        _ => d,
    });
    assert_eq!(depth, 0);
}

#[test]
fn plan_maps_onto_writer_input_and_repack_for_updates() {
    let temp = TempDir::new();
    let base_entries = title_entries(&TitleSpec::application(APP, 0, 1));
    temp.write("src/base.nsp", &dnsp(&base_entries));
    temp.write(
        "src/update.nsp",
        &dnsp(&title_entries(&TitleSpec::patch(APP, 0x10000, 2))),
    );
    let inv = run(&temp.path().join("src"), &fast());
    let plan = inv.plan();
    let app = &plan.applications[0];
    let options = WriterOptions::default();

    // Stage 1: bases (and DLC) have no unresolved dependency.
    let archive = temp.path().join("game.nx");
    let base_input = app
        .to_application_input(&|s| s.snapshot_type == SnapshotType::Base, &|_| None)
        .unwrap();
    write_archive(ArchiveInput::new(vec![base_input]), &archive, &options).unwrap();
    let summary = inspect_archive(&archive, &options, IntegrityPolicy::Strict).unwrap();
    let base_identity = summary.applications[0].snapshots[0].identity;
    assert_eq!(summary.applications[0].platform, Platform::Switch1);

    // Unresolved dependencies are refused rather than invented.
    assert!(app
        .to_application_input(&|s| s.snapshot_type == SnapshotType::Update, &|_| None)
        .is_err());

    // Stage 2: add the update with its packed base identity.
    let update_input = app
        .to_application_input(&|s| s.snapshot_type == SnapshotType::Update, &|r| {
            (r.snapshot_type == SnapshotType::Base).then_some(base_identity)
        })
        .unwrap();
    repack_archive(
        &archive,
        &archive,
        RepackPlan {
            remove_snapshots: vec![],
            additions: ArchiveInput::new(vec![update_input]),
            metadata_sources: vec![],
        },
        &options,
        IntegrityPolicy::Strict,
    )
    .unwrap();
    let summary = inspect_archive(&archive, &options, IntegrityPolicy::Strict).unwrap();
    let snaps = &summary.applications[0].snapshots;
    assert_eq!(snaps.len(), 2);
    let update = snaps
        .iter()
        .find(|s| s.snapshot_type == SnapshotType::Update)
        .unwrap();
    assert_eq!(update.numeric_version, 0x10000);
    assert_eq!(update.build_id, Some([2; 32]));
    // Base units are shared with the update view and deduplicated.
    let base_bytes: u64 = base_entries.iter().map(|e| e.1.len() as u64).sum();
    assert!(fs::metadata(&archive).unwrap().len() < base_bytes * 3);
}

#[test]
fn conversion_matrix_preserves_versions_and_selects_latest_metadata() {
    use nxformat_packer::conversion::convert_inventory;
    for (updates, dlcs) in [(0, 0), (1, 0), (3, 0), (0, 2), (3, 2)] {
        let temp = TempDir::new();
        temp.write(
            "src/base.nsp",
            &dnsp(&title_entries(&TitleSpec::application(APP, 0, 1))),
        );
        for i in 1..=updates {
            let mut spec = TitleSpec::patch(APP, i, (i + 1) as u8);
            spec.control.as_mut().unwrap().title = format!("Version {i}");
            if i == 2 {
                spec.program = None;
                spec.data = Some(vec![42; 8192]);
            }
            if i == 3 {
                spec.data = Some(vec![43; 8192]);
                spec.program.as_mut().unwrap().bktr = false;
            }
            temp.write(&format!("src/update{i}.nsp"), &dnsp(&title_entries(&spec)));
        }
        for i in 1..=dlcs {
            temp.write(
                &format!("src/dlc{i}.nsp"),
                &dnsp(&title_entries(&TitleSpec::add_on(
                    APP,
                    APP + 0x1000 + i,
                    0,
                    (i + 10) as u8,
                ))),
            );
        }
        let inv = run(&temp.path().join("src"), &strict());
        let out = temp.path().join("game.nx");
        convert_inventory(
            &inv,
            &out,
            &strict(),
            IntegrityPolicy::Strict,
            &WriterOptions::default(),
        )
        .unwrap();
        let summary =
            inspect_archive(&out, &WriterOptions::default(), IntegrityPolicy::Strict).unwrap();
        let app = &summary.applications[0];
        assert_eq!(app.snapshots.len(), 1 + updates as usize + dlcs as usize);
        let selected = app
            .snapshots
            .iter()
            .find(|s| Some(s.identity) == app.metadata_snapshot_identity)
            .unwrap();
        assert_eq!(selected.numeric_version, updates);
        if updates >= 2 {
            let data_only = app
                .snapshots
                .iter()
                .find(|s| s.snapshot_type == SnapshotType::Update && s.numeric_version == 2)
                .unwrap();
            assert_eq!(data_only.program_id, Some(APP));
            assert_eq!(data_only.build_id, Some([1; 32]));
        }
        let base = app
            .snapshots
            .iter()
            .find(|s| s.snapshot_type == SnapshotType::Base)
            .unwrap();
        let references = nxformat_packer::inspect_snapshot_files(
            &out,
            base.identity,
            &WriterOptions::default(),
            IntegrityPolicy::Fast,
        )
        .unwrap();
        assert_eq!(references.len(), 3);
        assert!(references.iter().all(|f| !f.blocks.is_empty()));
        let copy = temp.path().join("copy.nx");
        repack_archive(
            &out,
            &copy,
            RepackPlan::new(),
            &WriterOptions::default(),
            IntegrityPolicy::Strict,
        )
        .unwrap();
        assert_eq!(
            summary,
            inspect_archive(&copy, &WriterOptions::default(), IntegrityPolicy::Strict).unwrap()
        );
        assert_eq!(
            references,
            nxformat_packer::inspect_snapshot_files(
                &copy,
                base.identity,
                &WriterOptions::default(),
                IntegrityPolicy::Fast
            )
            .unwrap()
        );
    }
}

#[test]
fn conversion_revalidates_and_preserves_sources_destination_and_staging() {
    use nxformat_packer::conversion::convert_inventory;
    let temp = TempDir::new();
    let bytes = dnsp(&title_entries(&TitleSpec::application(APP, 0, 1)));
    let source = temp.write("src/base.nsp", &bytes);
    let inv = run(&temp.path().join("src"), &strict());
    let out = temp.write("old.nx", b"previous destination");
    let options = WriterOptions {
        max_temporary_disk_bytes: 256,
        ..WriterOptions::default()
    };
    assert!(convert_inventory(&inv, &out, &strict(), IntegrityPolicy::Strict, &options).is_err());
    assert_eq!(fs::read(&out).unwrap(), b"previous destination");
    assert_eq!(fs::read(&source).unwrap(), bytes);
    let options = WriterOptions::default();
    options.cancellation.cancel();
    let error =
        convert_inventory(&inv, &out, &strict(), IntegrityPolicy::Strict, &options).unwrap_err();
    assert_eq!(error.kind(), nxformat_packer::WriterErrorKind::Cancelled);
    assert_eq!(fs::read(&out).unwrap(), b"previous destination");
    assert!(convert_inventory(
        &inv,
        &source,
        &strict(),
        IntegrityPolicy::Strict,
        &WriterOptions::default()
    )
    .is_err());
    fs::write(&source, &bytes[..bytes.len() / 2]).unwrap();
    assert!(convert_inventory(
        &inv,
        &out,
        &strict(),
        IntegrityPolicy::Strict,
        &WriterOptions::default()
    )
    .is_err());
    assert_eq!(fs::read(&out).unwrap(), b"previous destination");
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
}

#[test]
fn metadata_mapping_keeps_exact_native_bytes_and_absent_values() {
    use std::io::Read;
    let temp = TempDir::new();
    let mut spec = TitleSpec::application(APP, 0, 1);
    spec.control.as_mut().unwrap().icon = b"\x89PNG\r\n\x1a\nexact native bytes".to_vec();
    spec.control.as_mut().unwrap().title.clear();
    let path = temp.write("base.nsp", &dnsp(&title_entries(&spec)));
    let inv = run(temp.path(), &strict());
    let plan = inv.plan();
    let s = &plan.applications[0].snapshots[0];
    let mut metadata = s.to_snapshot_input(None).unwrap().metadata.unwrap();
    assert!(metadata.title.is_none());
    assert_eq!(metadata.publisher.as_deref(), Some("NX tests"));
    assert_eq!(metadata.localized[0].language, "en-us");
    assert!(metadata.localized[0].title.is_none());
    let mut bytes = Vec::new();
    metadata.localized[0]
        .icon
        .as_mut()
        .unwrap()
        .reader
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, spec.control.unwrap().icon);
    let mut nacp = Vec::new();
    metadata
        .nacp
        .as_mut()
        .unwrap()
        .reader
        .read_to_end(&mut nacp)
        .unwrap();
    assert_eq!(
        nacp,
        inv.sources[0].metas[0]
            .meta
            .control
            .as_ref()
            .unwrap()
            .nacp_bytes
    );
    assert_eq!(&nacp[0x200..0x208], b"NX tests");
    assert_eq!(nxformat_packer::native::NACP_LANGUAGE_MAP.len(), 16);
    fs::remove_file(path).unwrap();
}

#[test]
fn update_uses_the_base_actually_accepted_by_the_plan() {
    let temp = TempDir::new();
    let bytes = dnsp(&title_entries(&TitleSpec::application(APP, 0, 1)));
    temp.write("a.nsp", &bytes);
    temp.write("b.nsp", &bytes);
    temp.write("u.nsp", &dnsp(&title_entries(&TitleSpec::patch(APP, 1, 2))));
    let mut inv = run(temp.path(), &fast());
    inv.sources[0].metas[0].plannable = false;
    let plan = inv.plan();
    let app = &plan.applications[0];
    let base = app
        .snapshots
        .iter()
        .find(|s| s.snapshot_type == SnapshotType::Base)
        .unwrap();
    let update = app
        .snapshots
        .iter()
        .find(|s| s.snapshot_type == SnapshotType::Update)
        .unwrap();
    assert!(base.files.iter().all(|f| f.source.ends_with("b.nsp")));
    assert!(base.files.iter().all(|f| update
        .files
        .iter()
        .any(|u| u.name == f.name && u.source == f.source)));
}

#[test]
fn initial_conversion_rejects_a_conflicting_native_base_patch_id() {
    let temp = TempDir::new();
    let mut base = TitleSpec::application(APP, 0, 1);
    base.related_id ^= 0x10;
    temp.write("base.nsp", &dnsp(&title_entries(&base)));
    temp.write(
        "update.nsp",
        &dnsp(&title_entries(&TitleSpec::patch(APP, 1, 2))),
    );
    let inv = run(temp.path(), &strict());
    let plan = inv.plan();
    assert!(plan
        .blocked
        .iter()
        .any(|b| b.reasons.iter().any(|r| r.contains("Base Patch ID"))));
    let out = temp.path().join("destination.nx");
    fs::write(&out, b"preserve destination").unwrap();
    assert!(nxformat_packer::conversion::convert_inventory(
        &inv,
        &out,
        &strict(),
        IntegrityPolicy::Strict,
        &WriterOptions::default()
    )
    .is_err());
    assert_eq!(fs::read(out).unwrap(), b"preserve destination");
}

#[test]
fn multiple_program_ncas_are_preserved_but_ambiguous_identical_ids_are_rejected() {
    let temp = TempDir::new();
    for same_id in [false, true] {
        let mut spec = TitleSpec::application(APP, 0, 1);
        let mut extra = spec.program.clone().unwrap();
        if !same_id {
            extra.program_id += 1;
        }
        extra.build_id = [7; 32];
        spec.extra_programs.push(extra);
        let source = temp.write("multi.nsp", &dnsp(&title_entries(&spec)));
        let inv = nxformat_packer::native::associate(vec![inspect_source(&source, &strict())]);
        assert_eq!(inv.sources[0].metas[0].meta.programs.len(), 2);
        assert_eq!(
            inv.sources[0].metas[0].meta.programs[1].build_id,
            Some([7; 32])
        );
        assert_eq!(inv.sources[0].metas[0].meta.programs[1].id_offset, 1);
        if same_id {
            assert!(inv.plan().applications.is_empty());
            assert!(inv.plan().blocked[0]
                .reasons
                .iter()
                .any(|r| r.contains("ambiguous duplicate Program")));
        } else {
            let plan = inv.plan();
            assert_eq!(plan.applications[0].snapshots.len(), 2);
            assert_eq!(plan.applications[0].snapshots[1].program_id, Some(APP + 1));
            assert_eq!(plan.applications[0].snapshots[1].build_id, Some([7; 32]));
        }
    }
    let versions = TempDir::new();
    let mut first = TitleSpec::application(APP, 0, 1);
    first.control = None;
    let mut extra = first.program.clone().unwrap();
    extra.program_id = APP + 1;
    first.extra_programs.push(extra);
    versions.write("base-0.nsp", &dnsp(&title_entries(&first)));
    let mut second = first.clone();
    second.version = 1;
    second.seed = 55;
    versions.write(
        "base-1.nsp",
        &dnsp(&nxformat_packer::native::synthetic::title_entries_custom(
            &second,
            &[1, 0],
            &[],
        )),
    );
    let changed = run(versions.path(), &strict());
    assert!(changed.plan().blocked.iter().any(|b| b
        .reasons
        .iter()
        .any(|r| r.contains("changing primary Program roles"))));
}

#[test]
fn all_legacy_nacp_languages_map_without_inventing_publishers_or_icons() {
    let temp = TempDir::new();
    let mut spec = TitleSpec::application(APP, 0, 1);
    let c = spec.control.as_mut().unwrap();
    for i in 1..16 {
        c.localized
            .push((i, format!("Title {i}"), format!("Publisher {i}")));
    }
    c.extra_icons
        .push(("UnknownLanguage".into(), b"unknown native icon".to_vec()));
    let source = temp.write("all.nsp", &dnsp(&title_entries(&spec)));
    let inv = nxformat_packer::native::associate(vec![inspect_source(&source, &strict())]);
    let mapped = inv.plan().applications[0].snapshots[0]
        .to_snapshot_input(None)
        .unwrap()
        .metadata
        .unwrap();
    assert_eq!(mapped.localized.len(), 16);
    for (index, entry) in mapped.localized.iter().enumerate() {
        assert_eq!(
            entry.language,
            nxformat_packer::native::NACP_LANGUAGE_MAP[index].1
        );
        assert_eq!(entry.icon.is_some(), index == 0);
    }
    spec.control.as_mut().unwrap().titles_format = 1;
    fs::write(&source, dnsp(&title_entries(&spec))).unwrap();
    let inv = nxformat_packer::native::associate(vec![inspect_source(&source, &strict())]);
    let control = inv.sources[0].metas[0].meta.control.as_ref().unwrap();
    assert!(control.titles.is_empty());
    assert_eq!(control.nacp_bytes[0x3215], 1);
    assert!(has(&inv, Severity::Warning, IssueCode::MetadataUnavailable));
}

#[test]
fn every_converted_native_unit_reads_identically_through_rust_block_references() {
    use std::io::{Read, Seek, SeekFrom};
    let temp = TempDir::new();
    temp.write(
        "src/base.nsp",
        &dnsp(&title_entries(&TitleSpec::application(APP, 0, 1))),
    );
    temp.write(
        "src/update.nsp",
        &dnsp(&title_entries(&TitleSpec::patch(APP, 1, 2))),
    );
    let inv = run(&temp.path().join("src"), &strict());
    let out = temp.path().join("out.nx");
    nxformat_packer::conversion::convert_inventory(
        &inv,
        &out,
        &strict(),
        IntegrityPolicy::Strict,
        &WriterOptions::default(),
    )
    .unwrap();
    let summary =
        inspect_archive(&out, &WriterOptions::default(), IntegrityPolicy::Strict).unwrap();
    let plan = inv.plan();
    for snapshot in &summary.applications[0].snapshots {
        let native = plan.applications[0]
            .snapshots
            .iter()
            .find(|s| s.content_id == snapshot.content_id)
            .unwrap();
        let refs = nxformat_packer::inspect_snapshot_files(
            &out,
            snapshot.identity,
            &WriterOptions::default(),
            IntegrityPolicy::Strict,
        )
        .unwrap();
        assert_eq!(refs.len(), native.files.len());
        for file in refs {
            let expected = native.files.iter().find(|f| f.name == file.path).unwrap();
            let mut input = fs::File::open(&expected.source).unwrap();
            input.seek(SeekFrom::Start(expected.offset)).unwrap();
            let mut expected_bytes = Vec::new();
            input
                .take(expected.size)
                .read_to_end(&mut expected_bytes)
                .unwrap();
            let mut packed = fs::File::open(&out).unwrap();
            let mut actual = Vec::new();
            for block in file.blocks {
                packed.seek(SeekFrom::Start(block.offset)).unwrap();
                let mut stored = vec![0; block.stored_size as usize];
                packed.read_exact(&mut stored).unwrap();
                if block.codec == 0 {
                    actual.extend(stored);
                } else {
                    actual.extend(
                        zstd::bulk::decompress(&stored, block.logical_size as usize).unwrap(),
                    );
                }
            }
            assert_eq!(actual, expected_bytes);
        }
    }
}

#[test]
fn conversion_update_staging_and_final_publication_fail_without_damage() {
    let temp = TempDir::new();
    temp.write(
        "src/base.nsp",
        &dnsp(&title_entries(&TitleSpec::application(APP, 0, 1))),
    );
    let mut update = TitleSpec::patch(APP, 1, 2);
    update.program.as_mut().unwrap().romfs_bytes = 2 * 1024 * 1024;
    temp.write("src/update.nsp", &dnsp(&title_entries(&update)));
    let inv = run(&temp.path().join("src"), &strict());
    let out = temp.write("old.nx", b"previous destination");
    let options = WriterOptions {
        max_temporary_disk_bytes: 256 * 1024,
        ..WriterOptions::default()
    };
    let error = nxformat_packer::conversion::convert_inventory(
        &inv,
        &out,
        &strict(),
        IntegrityPolicy::Strict,
        &options,
    )
    .unwrap_err();
    assert_eq!(
        error.kind(),
        nxformat_packer::WriterErrorKind::ResourceLimit
    );
    assert_eq!(fs::read(&out).unwrap(), b"previous destination");
    let directory = temp.path().join("destination");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("keep"), b"keep").unwrap();
    assert!(nxformat_packer::conversion::convert_inventory(
        &inv,
        &directory,
        &strict(),
        IntegrityPolicy::Strict,
        &WriterOptions::default()
    )
    .is_err());
    assert_eq!(fs::read(directory.join("keep")).unwrap(), b"keep");
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 3);
}

#[test]
fn missing_update_control_does_not_inherit_base_display_metadata() {
    let temp = TempDir::new();
    temp.write(
        "base.nsp",
        &dnsp(&title_entries(&TitleSpec::application(APP, 0, 1))),
    );
    let mut update = TitleSpec::patch(APP, 1, 2);
    update.control = None;
    temp.write("update.nsp", &dnsp(&title_entries(&update)));
    let inv = run(temp.path(), &strict());
    let plan = inv.plan();
    let update = plan.applications[0]
        .snapshots
        .iter()
        .find(|s| s.snapshot_type == SnapshotType::Update)
        .unwrap();
    assert!(update.display_version.is_none());
    assert!(update
        .to_snapshot_input(Some([1; 32]))
        .unwrap()
        .metadata
        .is_none());
}

#[test]
fn collection_with_two_authoritative_application_metas_keeps_both_games() {
    let temp = TempDir::new();
    let mut entries = Vec::new();
    for (app, seed) in [(APP, 1), (APP + 0x2000, 2)] {
        entries.extend(title_entries(&TitleSpec::application(app, 0, seed)));
        entries.extend(title_entries(&TitleSpec::patch(app, 1, seed + 3)));
    }
    temp.write("collection.bin", &dnsp(&entries));
    let inv = run(temp.path(), &strict());
    assert_eq!(inv.applications.len(), 2);
    let out = temp.path().join("collection.nx");
    nxformat_packer::conversion::convert_inventory(
        &inv,
        &out,
        &strict(),
        IntegrityPolicy::Strict,
        &WriterOptions::default(),
    )
    .unwrap();
    let summary = inspect_archive(out, &WriterOptions::default(), IntegrityPolicy::Strict).unwrap();
    assert_eq!(summary.applications.len(), 2);
    for app in summary.applications {
        assert_eq!(app.programs.len(), 1);
        assert_eq!(app.snapshots.len(), 2);
        assert!(app
            .snapshots
            .iter()
            .all(|s| s.program_id == Some(app.application_id)));
    }
}

#[test]
fn extracted_full_tree_under_update_folder_has_facts_but_no_inferred_identity() {
    let temp = TempDir::new();
    temp.write("updates/917504/exefs/main.npdm", &npdm(APP));
    let mut nso = vec![0u8; 0x100];
    nso[..4].copy_from_slice(b"NSO0");
    nso[0x40..0x60].copy_from_slice(&[7; 32]);
    temp.write("updates/917504/exefs/main", &nso);
    temp.write("updates/917504/exefs/subsdk0", &nso);
    temp.write("updates/917504/romfs/complete-game.bin", b"native data");
    let inv = run(temp.path(), &strict());
    assert_eq!(inv.sources.len(), 1);
    let tree = inv.sources[0].tree.as_ref().unwrap();
    assert!(tree.has_romfs);
    assert_eq!(tree.program_id, Some(APP));
    assert_eq!(tree.nso_build_ids.len(), 2);
    assert_eq!(tree.main_build_id, Some([7; 32]));
    assert!(inv.applications.is_empty());
    assert!(inv.plan().applications.is_empty());
    assert!(!inv.all_issues().any(|i| i.code == IssueCode::MissingBase));
    let out = temp.write("previous.nx", b"previous");
    assert!(nxformat_packer::conversion::convert_inventory(
        &inv,
        &out,
        &strict(),
        IntegrityPolicy::Strict,
        &WriterOptions::default()
    )
    .is_err());
    assert_eq!(fs::read(out).unwrap(), b"previous");
}

#[test]
fn update_with_changed_program_association_is_not_treated_as_data_only() {
    let temp = TempDir::new();
    temp.write(
        "base.nsp",
        &dnsp(&title_entries(&TitleSpec::application(APP, 0, 1))),
    );
    let mut update = TitleSpec::patch(APP, 1, 2);
    update.program.as_mut().unwrap().program_id = APP + 1;
    temp.write("update.nsp", &dnsp(&title_entries(&update)));
    let inv = run(temp.path(), &strict());
    let plan = inv.plan();
    assert_eq!(plan.applications[0].snapshots.len(), 1);
    assert!(plan
        .blocked
        .iter()
        .any(|b| b.reasons.iter().any(|r| r.contains("Program ID differs"))));
}

#[test]
fn dlc_numeric_requirement_binds_to_an_exact_accepted_update() {
    let temp = TempDir::new();
    temp.write(
        "src/base.nsp",
        &dnsp(&title_entries(&TitleSpec::application(APP, 0, 1))),
    );
    let mut dlc = TitleSpec::add_on(APP, APP + 0x1001, 0, 3);
    dlc.required_application_version = 2;
    temp.write("src/dlc.nsp", &dnsp(&title_entries(&dlc)));
    let inv = run(&temp.path().join("src"), &strict());
    assert!(inv.plan().blocked.iter().any(|b| b
        .reasons
        .iter()
        .any(|r| r.contains("DLC requires application version 2"))));
    temp.write(
        "src/update.nsp",
        &dnsp(&title_entries(&TitleSpec::patch(APP, 2, 2))),
    );
    let inv = run(&temp.path().join("src"), &strict());
    let plan = inv.plan();
    let dlc = plan.applications[0]
        .snapshots
        .iter()
        .find(|s| s.snapshot_type == SnapshotType::Dlc)
        .unwrap();
    assert_eq!(
        dlc.requires.as_ref().unwrap().snapshot_type,
        SnapshotType::Update
    );
    let out = temp.path().join("game.nx");
    let stats = nxformat_packer::conversion::convert_inventory(
        &inv,
        &out,
        &strict(),
        IntegrityPolicy::Strict,
        &WriterOptions::default(),
    )
    .unwrap();
    assert_eq!(stats.additions.len(), 2);
    let summary =
        inspect_archive(&out, &WriterOptions::default(), IntegrityPolicy::Strict).unwrap();
    let update = summary.applications[0]
        .snapshots
        .iter()
        .find(|s| s.snapshot_type == SnapshotType::Update)
        .unwrap();
    let original = fs::read(&out).unwrap();
    let mut remove = RepackPlan::new();
    remove.remove_snapshots.push(update.identity);
    assert!(repack_archive(
        &out,
        &out,
        remove,
        &WriterOptions::default(),
        IntegrityPolicy::Strict
    )
    .is_err());
    assert_eq!(original, fs::read(&out).unwrap());
}
