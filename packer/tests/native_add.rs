//! Existing-NX additions; all sources are deterministic synthetic native data.
use nxformat_packer::native::synthetic::*;
use nxformat_packer::native::*;
use nxformat_packer::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const APP: u64 = 0x0100_0000_0001_0000;
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "nx-add-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn native(&self, name: &str, spec: &TitleSpec) -> PathBuf {
        let p = self.path(name);
        fs::write(&p, dnsp(&title_entries(spec))).unwrap();
        p
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn native_options() -> InventoryOptions {
    InventoryOptions {
        mode: InventoryMode::Strict,
        ..InventoryOptions::default()
    }
}
fn base(temp: &Temp, options: &WriterOptions) -> PathBuf {
    let p = temp.native("base.nsp", &TitleSpec::application(APP, 0, 1));
    let inv = associate(vec![inspect_source(&p, &native_options())]);
    let nx = temp.path("base.nx");
    nxformat_packer::conversion::convert_inventory(
        &inv,
        &nx,
        &native_options(),
        IntegrityPolicy::Strict,
        options,
    )
    .unwrap();
    fs::remove_file(p).unwrap(); // No original native base remains.
    nx
}
fn summary(path: &Path, options: &WriterOptions) -> ArchiveSummary {
    inspect_archive(path, options, IntegrityPolicy::Strict).unwrap()
}

fn compare_files(
    path: &Path,
    id: [u8; 32],
    entries: &[(String, Vec<u8>)],
    options: &WriterOptions,
) {
    let references = inspect_snapshot_files(path, id, options, IntegrityPolicy::Fast).unwrap();
    assert_eq!(references.len(), entries.len());
    for (name, bytes) in entries {
        assert_eq!(
            references.iter().find(|r| r.path == *name).unwrap().size,
            bytes.len() as u64
        );
        let mut offset = 0;
        while offset < bytes.len() {
            let end = (offset + 65519).min(bytes.len());
            let mut buffer = vec![0; end - offset];
            read_snapshot_file_range(path, id, name, offset as u64, &mut buffer, options).unwrap();
            assert_eq!(buffer, bytes[offset..end]);
            offset = end;
        }
    }
}

fn compare_retained_frames(before: &Path, after: &Path, options: &WriterOptions) {
    let original = summary(before, options);
    let old = fs::read(before).unwrap();
    let new = fs::read(after).unwrap();
    for snapshot in &original.applications[0].snapshots {
        let a = inspect_snapshot_files(before, snapshot.identity, options, IntegrityPolicy::Fast)
            .unwrap();
        let b = inspect_snapshot_files(after, snapshot.identity, options, IntegrityPolicy::Fast)
            .unwrap();
        for file in a {
            let kept = b.iter().find(|f| f.path == file.path).unwrap();
            assert_eq!(file.digest, kept.digest);
            assert_eq!(file.blocks.len(), kept.blocks.len());
            for (x, y) in file.blocks.iter().zip(&kept.blocks) {
                assert_eq!(
                    (x.codec, &x.checks, x.logical_size),
                    (y.codec, &y.checks, y.logical_size)
                );
                assert_eq!(
                    &old[x.offset as usize..(x.offset + x.stored_size as u64) as usize],
                    &new[y.offset as usize..(y.offset + y.stored_size as u64) as usize]
                );
            }
        }
    }
}

#[test]
fn native_batch_publishes_updates_and_same_version_dlcs_once() {
    let temp = Temp::new();
    let options = WriterOptions::default();
    let nx = base(&temp, &options);
    let update1 = TitleSpec::patch(APP, 1, 21);
    let update2 = TitleSpec::patch(APP, 2, 22);
    let dlc1 = TitleSpec::add_on(APP, APP + 0x1001, 0, 41);
    let dlc2 = TitleSpec::add_on(APP, APP + 0x1002, 0, 42);
    let sources = [
        temp.native("batch-u1.nsp", &update1),
        temp.native("batch-u2.nsp", &update2),
        temp.native("batch-d1.nsp", &dlc1),
        temp.native("batch-d2.nsp", &dlc2),
    ];
    let requests = [
        NativeAddRequest::new(&sources[0], APP | 0x800, 1),
        NativeAddRequest::new(&sources[1], APP | 0x800, 2),
        NativeAddRequest::new(&sources[2], APP + 0x1001, 0),
        NativeAddRequest::new(&sources[3], APP + 0x1002, 0),
    ];
    let stats = add_native_contents(
        &nx,
        &nx,
        &requests,
        &native_options(),
        &options,
        IntegrityPolicy::Strict,
    )
    .unwrap();
    assert!(stats.repack.in_place_append);
    assert_eq!(stats.repack.added_snapshots, 4);
    assert_eq!(stats.added_snapshot_identities.len(), 4);
    let output = summary(&nx, &options);
    let snapshots = &output.applications[0].snapshots;
    assert_eq!(snapshots.len(), 5);
    for (spec, ty, version) in [
        (&update1, SnapshotType::Update, 1),
        (&update2, SnapshotType::Update, 2),
        (&dlc1, SnapshotType::Dlc, 0),
        (&dlc2, SnapshotType::Dlc, 0),
    ] {
        let expected = title_entries(spec);
        let matches: Vec<_> = snapshots
            .iter()
            .filter(|s| s.snapshot_type == ty && s.numeric_version == version)
            .collect();
        let mut found = 0;
        for snapshot in matches {
            let files =
                inspect_snapshot_files(&nx, snapshot.identity, &options, IntegrityPolicy::Fast)
                    .unwrap();
            if ty == SnapshotType::Dlc {
                let actual_names: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
                let expected_names: Vec<_> =
                    expected.iter().map(|(name, _)| name.as_str()).collect();
                if actual_names.len() == expected_names.len()
                    && actual_names.iter().all(|n| expected_names.contains(n))
                {
                    compare_files(&nx, snapshot.identity, &expected, &options);
                    found += 1;
                }
            } else {
                let mut expected_with_base = title_entries(&TitleSpec::application(APP, 0, 1));
                expected_with_base.extend(expected.clone());
                compare_files(&nx, snapshot.identity, &expected_with_base, &options);
                found += 1;
            }
        }
        assert_eq!(found, 1, "one matching native snapshot should exist");
    }
    // Exactly one generation was committed for the four additions.
    let headers = fs::read(&nx).unwrap();
    let generation =
        |offset: usize| u64::from_le_bytes(headers[offset + 216..offset + 224].try_into().unwrap());
    assert_eq!(generation(0).max(generation(4096)), 1);

    // A bad second member cannot publish a valid first member.
    let before = fs::read(&nx).unwrap();
    let extra = temp.native(
        "batch-failed-valid.nsp",
        &TitleSpec::add_on(APP, APP + 0x1003, 0, 43),
    );
    let error = add_native_contents(
        &nx,
        &nx,
        &[
            NativeAddRequest::new(extra, APP + 0x1003, 0),
            NativeAddRequest::new(&sources[2], APP + 0x9999, 0),
        ],
        &native_options(),
        &options,
        IntegrityPolicy::Fast,
    )
    .unwrap_err();
    assert_eq!(error.kind(), WriterErrorKind::InvalidInput);
    assert_eq!(fs::read(&nx).unwrap(), before);
    assert!(add_native_contents(
        &nx,
        &nx,
        &[],
        &native_options(),
        &options,
        IntegrityPolicy::Fast
    )
    .is_err());

    // Removing both independent version-zero DLCs is already a single
    // transaction in the existing RepackPlan API.
    let mut removal = RepackPlan::new();
    removal.remove_snapshots = snapshots
        .iter()
        .filter(|s| s.snapshot_type == SnapshotType::Dlc)
        .map(|s| s.identity)
        .collect();
    let removed = repack_archive(&nx, &nx, removal, &options, IntegrityPolicy::Strict).unwrap();
    assert_eq!(removed.removed_snapshots, 2);
    let remaining = summary(&nx, &options);
    assert_eq!(remaining.applications[0].snapshots.len(), 3);
}

#[test]
fn native_batch_keeps_multi_program_update_families_separate() {
    let temp = Temp::new();
    let options = WriterOptions::default();
    let mut base_spec = TitleSpec::application(APP, 0, 1);
    let mut other = base_spec.program.clone().unwrap();
    other.program_id = APP + 1;
    other.build_id = [7; 32];
    base_spec.extra_programs.push(other);
    let original_files = title_entries(&base_spec);
    let base_source = temp.native("multi-base.nsp", &base_spec);
    let nx = temp.path("multi.nx");
    let inventory = associate(vec![inspect_source(&base_source, &native_options())]);
    nxformat_packer::conversion::convert_inventory(
        &inventory,
        &nx,
        &native_options(),
        IntegrityPolicy::Strict,
        &options,
    )
    .unwrap();
    let mut first = TitleSpec::patch(APP, 1, 31);
    first.program.as_mut().unwrap().program_id = APP + 1;
    first.control = None;
    let first_files = title_entries_custom(&first, &[1], &[]);
    let first_path = temp.path("multi-update1.nsp");
    fs::write(&first_path, dnsp(&first_files)).unwrap();
    let mut second = TitleSpec::patch(APP, 2, 32);
    let mut secondary = second.program.clone().unwrap();
    secondary.program_id = APP + 1;
    secondary.build_id = [99; 32];
    second.extra_programs.push(secondary);
    let second_files = title_entries(&second);
    let second_path = temp.path("multi-update2.nsp");
    fs::write(&second_path, dnsp(&second_files)).unwrap();
    let stats = add_native_contents(
        &nx,
        &nx,
        &[
            NativeAddRequest::new(first_path, APP | 0x800, 1),
            NativeAddRequest::new(second_path, APP | 0x800, 2),
        ],
        &native_options(),
        &options,
        IntegrityPolicy::Strict,
    )
    .unwrap();
    assert_eq!(stats.repack.added_snapshots, 4);
    assert_eq!(stats.added_snapshot_identities.len(), 4);
    let snapshots = summary(&nx, &options).applications.remove(0).snapshots;
    for (version, files) in [(1, first_files), (2, second_files)] {
        let mut expected = original_files.clone();
        expected.extend(files);
        for program in [APP, APP + 1] {
            let snapshot = snapshots
                .iter()
                .find(|s| s.numeric_version == version && s.program_id == Some(program))
                .unwrap();
            compare_files(&nx, snapshot.identity, &expected, &options);
        }
    }
}

#[test]
fn sequential_bktr_executable_data_only_and_dlc_without_original_base() {
    let temp = Temp::new();
    let options = WriterOptions::default();
    let original = base(&temp, &options);
    let original_bytes = fs::read(&original).unwrap();
    let initial = summary(&original, &options);
    let base_id = initial.applications[0].snapshots[0].identity;
    let mut current = original.clone();
    for version in 1..=3 {
        let mut spec = TitleSpec::patch(APP, version, 10 + version as u8);
        if version == 1 {
            spec.data = Some(vec![0x66; 9999]);
        }
        if version == 2 {
            spec.program.as_mut().unwrap().bktr = false;
        }
        if version == 3 {
            spec.program = None;
            spec.data = Some(vec![7; 12345]);
        }
        let p = temp.native(&format!("u{version}.nsp"), &spec);
        let out = temp.path(&format!("u{version}.nx"));
        let stats = add_native_content(
            &current,
            &out,
            &NativeAddRequest::new(&p, APP | 0x800, version),
            &native_options(),
            &options,
            IntegrityPolicy::Strict,
        )
        .unwrap();
        assert_eq!(stats.base_snapshot_identity, base_id);
        assert_eq!(stats.required_snapshot_identity, base_id);
        assert!(stats.inherited_file_bytes > 0);
        assert!(
            stats.new_native_bytes_staged
                < stats.inherited_file_bytes + stats.new_native_bytes_staged
        );
        let result = summary(&out, &options);
        let s = result.applications[0]
            .snapshots
            .iter()
            .find(|s| s.identity == stats.added_snapshot_identity)
            .unwrap();
        assert_eq!(s.program_id, Some(APP));
        assert_eq!(
            s.build_id,
            Some([if version == 3 { 1 } else { 10 + version as u8 }; 32])
        );
        assert_eq!(
            result.applications[0].metadata_snapshot_identity,
            Some(s.identity)
        );
        let mut expected = title_entries(&TitleSpec::application(APP, 0, 1));
        expected.extend(title_entries(&spec));
        compare_files(&out, s.identity, &expected, &options);
        compare_retained_frames(&current, &out, &options);
        current = out;
    }
    let mut dlc = TitleSpec::add_on(APP, APP + 0x1001, 0, 88);
    dlc.required_application_version = 2;
    let p = temp.native("dlc.nsp", &dlc);
    let out = temp.path("dlc.nx");
    let stats = add_native_content(
        &current,
        &out,
        &NativeAddRequest::new(p, APP + 0x1001, 0),
        &native_options(),
        &options,
        IntegrityPolicy::Strict,
    )
    .unwrap();
    let before = summary(&current, &options);
    assert_eq!(
        stats.required_snapshot_identity,
        before.applications[0]
            .snapshots
            .iter()
            .find(|s| s.numeric_version == 2)
            .unwrap()
            .identity
    );
    assert_eq!(stats.inherited_files, 0);
    assert_eq!(summary(&out, &options).applications[0].snapshots.len(), 5);
    assert_eq!(fs::read(original).unwrap(), original_bytes);
}

#[test]
fn rejection_and_cancellation_preserve_source_destination_and_temps() {
    let temp = Temp::new();
    let options = WriterOptions::default();
    let nx = base(&temp, &options);
    let original = fs::read(&nx).unwrap();
    let out = temp.path("destination.nx");
    fs::write(&out, b"old").unwrap();
    let mut spec = TitleSpec::patch(APP, 1, 10);
    let p = temp.native("update.nsp", &spec);
    let mut request = NativeAddRequest::new(&p, APP | 0x800, 1);
    for case in 0..6 {
        let mut opts = WriterOptions::default();
        request.base_snapshot_identity = None;
        match case {
            0 => opts.cancellation.cancel(),
            1 => opts.max_temporary_disk_bytes = 100,
            2 => opts.max_memory_bytes = 100,
            3 => request.base_snapshot_identity = Some([99; 32]),
            4 => {
                spec.program.as_mut().unwrap().program_id = APP + 1;
                fs::write(&p, dnsp(&title_entries(&spec))).unwrap();
            }
            _ => {
                spec.program.as_mut().unwrap().program_id = APP;
                spec.extra_programs.push(spec.program.clone().unwrap());
                fs::write(&p, dnsp(&title_entries(&spec))).unwrap();
            }
        }
        let error = add_native_content(
            &nx,
            &out,
            &request,
            &native_options(),
            &opts,
            IntegrityPolicy::Strict,
        )
        .unwrap_err();
        assert_eq!(
            error.kind() == WriterErrorKind::Cancelled,
            case == 0,
            "case {case}"
        );
        assert_eq!(fs::read(&out).unwrap(), b"old");
        assert_eq!(fs::read(&nx).unwrap(), original);
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 3);
    }
}

#[test]
fn retained_native_units_are_referenced_without_restaging_and_mismatches_fail() {
    let temp = Temp::new();
    let options = WriterOptions::default();
    let nx = base(&temp, &options);
    let first = TitleSpec::patch(APP, 1, 10);
    let p = temp.native("u1.nsp", &first);
    let one = temp.path("one.nx");
    add_native_content(
        &nx,
        &one,
        &NativeAddRequest::new(&p, APP | 0x800, 1),
        &native_options(),
        &options,
        IntegrityPolicy::Strict,
    )
    .unwrap();
    let mut second = first.clone();
    second.version = 2;
    let mut entries = title_entries(&second);
    let meta = entries
        .iter_mut()
        .find(|(n, _)| n.ends_with(".cnmt.nca"))
        .unwrap();
    meta.0 = format!("{}.cnmt.nca", "aa".repeat(16));
    let meta_size = meta.1.len() as u64;
    let p = temp.path("u2.nsp");
    fs::write(&p, dnsp(&entries)).unwrap();
    let two = temp.path("two.nx");
    let stats = add_native_content(
        &one,
        &two,
        &NativeAddRequest::new(&p, APP | 0x800, 2),
        &native_options(),
        &options,
        IntegrityPolicy::Strict,
    )
    .unwrap();
    assert_eq!(stats.new_native_bytes_staged, meta_size);
    assert!(stats.new_source_equivalence_bytes_read > 0);
    assert!(stats.repack.added_logical_bytes_considered_for_compression <= meta_size);
    let mut expected = title_entries(&TitleSpec::application(APP, 0, 1));
    expected.extend(entries.clone());
    compare_files(&two, stats.added_snapshot_identity, &expected, &options);
    compare_retained_frames(&one, &two, &options);
    let program = entries.iter_mut().find(|(_, b)| b[0x205] == 0).unwrap();
    *program.1.last_mut().unwrap() ^= 1;
    fs::write(&p, dnsp(&entries)).unwrap();
    let before = fs::read(&two).unwrap();
    assert!(add_native_content(
        &one,
        &two,
        &NativeAddRequest::new(p, APP | 0x800, 2),
        &native_options(),
        &options,
        IntegrityPolicy::Strict
    )
    .is_err());
    assert_eq!(fs::read(&two).unwrap(), before);
}

#[test]
fn corrupt_truncated_sources_missing_native_content_and_publication_failure() {
    let temp = Temp::new();
    let options = WriterOptions::default();
    let nx = base(&temp, &options);
    let bytes = fs::read(&nx).unwrap();
    let p = temp.native("u.nsp", &TitleSpec::patch(APP, 1, 10));
    let request = NativeAddRequest::new(&p, APP | 0x800, 1);
    let out = temp.path("out.nx");
    fs::write(&out, b"old").unwrap();
    let base_id = summary(&nx, &options).applications[0].snapshots[0].identity;
    let refs = inspect_snapshot_files(&nx, base_id, &options, IntegrityPolicy::Fast).unwrap();
    let payload = refs[0].blocks[0].offset as usize;
    for case in 0..3 {
        let mut changed = bytes.clone();
        match case {
            0 => changed.truncate(bytes.len() - 1),
            1 => {
                // One damaged slot is tolerated; both must fail.
                changed[0] ^= 1;
                changed[4096] ^= 1;
            }
            _ => changed[payload] ^= 1,
        }
        fs::write(&nx, changed).unwrap();
        assert!(add_native_content(
            &nx,
            &out,
            &request,
            &native_options(),
            &options,
            IntegrityPolicy::Strict
        )
        .is_err());
        assert_eq!(fs::read(&out).unwrap(), b"old");
    }
    fs::write(&nx, &bytes).unwrap();
    let mut entries = title_entries(&TitleSpec::patch(APP, 1, 10));
    entries.retain(|(_, b)| b[0x205] != 0);
    fs::write(&p, dnsp(&entries)).unwrap();
    assert!(add_native_content(
        &nx,
        &out,
        &request,
        &native_options(),
        &options,
        IntegrityPolicy::Strict
    )
    .is_err());
    fs::write(&p, dnsp(&title_entries(&TitleSpec::patch(APP, 1, 10)))).unwrap();
    fs::remove_file(&out).unwrap();
    fs::create_dir(&out).unwrap();
    fs::write(out.join("sentinel"), b"old").unwrap();
    assert!(add_native_content(
        &nx,
        &out,
        &request,
        &native_options(),
        &options,
        IntegrityPolicy::Strict
    )
    .is_err());
    assert_eq!(fs::read(out.join("sentinel")).unwrap(), b"old");
    assert_eq!(fs::read(&nx).unwrap(), bytes);
    assert!(!fs::read_dir(&temp.0).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".nx-")));
}

#[test]
fn ambiguous_missing_wrong_native_bases_and_exact_selection() {
    let temp = Temp::new();
    let options = WriterOptions::default();
    let a = temp.native("b0.nsp", &TitleSpec::application(APP, 0, 1));
    let b = temp.native("b1.nsp", &TitleSpec::application(APP, 1, 2));
    let plan = associate(vec![
        inspect_source(&a, &native_options()),
        inspect_source(&b, &native_options()),
    ])
    .plan();
    let input = || {
        plan.applications[0]
            .to_application_input(&|_| true, &|_| None)
            .unwrap()
    };
    let nx = temp.path("bases.nx");
    write_archive(ArchiveInput::new(vec![input()]), &nx, &options).unwrap();
    fs::remove_file(a).unwrap();
    fs::remove_file(b).unwrap();
    let p = temp.native("u.nsp", &TitleSpec::patch(APP, 2, 10));
    let out = temp.path("out.nx");
    let mut request = NativeAddRequest::new(&p, APP | 0x800, 2);
    assert!(add_native_content(
        &nx,
        &out,
        &request,
        &native_options(),
        &options,
        IntegrityPolicy::Strict
    )
    .is_err());
    let id = summary(&nx, &options).applications[0]
        .snapshots
        .iter()
        .find(|s| s.numeric_version == 0)
        .unwrap()
        .identity;
    request.base_snapshot_identity = Some(id);
    add_native_content(
        &nx,
        &out,
        &request,
        &native_options(),
        &options,
        IntegrityPolicy::Strict,
    )
    .unwrap();
    // With several retained bases, DLC dependency selection must stay on the
    // explicitly selected base rather than picking a lower other base.
    let latest = summary(&nx, &options).applications[0]
        .snapshots
        .iter()
        .find(|s| s.numeric_version == 1)
        .unwrap()
        .identity;
    let dlc = temp.native("d.nsp", &TitleSpec::add_on(APP, APP + 0x1001, 0, 77));
    let mut dlc_request = NativeAddRequest::new(dlc, APP + 0x1001, 0);
    dlc_request.base_snapshot_identity = Some(latest);
    let stats = add_native_content(
        &nx,
        temp.path("d.nx"),
        &dlc_request,
        &native_options(),
        &options,
        IntegrityPolicy::Strict,
    )
    .unwrap();
    assert_eq!(stats.required_snapshot_identity, latest);
    let mut wrong_plan = plan.applications[0].clone();
    wrong_plan.snapshots.retain(|s| s.numeric_version == 0);
    wrong_plan.snapshots[0].files = Vec::new().into();
    wrong_plan.snapshots[0].control = None;
    let mut wrong = wrong_plan
        .to_application_input(&|_| true, &|_| None)
        .unwrap();
    wrong.snapshots[0].numeric_version = 99;
    // Reopen streams are intentionally unavailable: construct from generated
    // bytes instead of consulting the deleted original dump.
    let entries = title_entries(&TitleSpec::application(APP, 0, 1));
    wrong.snapshots[0].root = DirectoryInput::new(
        entries
            .into_iter()
            .map(|(n, b)| EntryInput::file(n, FileInput::from_bytes(b)))
            .collect(),
    );
    write_archive(ArchiveInput::new(vec![wrong]), &nx, &options).unwrap();
    request.base_snapshot_identity = None;
    assert!(add_native_content(
        &nx,
        &out,
        &request,
        &native_options(),
        &options,
        IntegrityPolicy::Strict
    )
    .is_err());
}

#[test]
fn dlc_title_versions_dependencies_and_native_source_alias_are_checked() {
    let temp = Temp::new();
    let options = WriterOptions::default();
    let nx = base(&temp, &options);
    let mut current = nx;
    for index in 1..=2 {
        let p = temp.native(
            &format!("d{index}.nsp"),
            &TitleSpec::add_on(APP, APP + 0x1000 + index, 0, 30 + index as u8),
        );
        let out = temp.path(&format!("d{index}.nx"));
        add_native_content(
            &current,
            &out,
            &NativeAddRequest::new(p, APP + 0x1000 + index, 0),
            &native_options(),
            &options,
            IntegrityPolicy::Strict,
        )
        .unwrap();
        current = out;
    }
    for (title, required) in [(APP + 0x1001, 0), (APP + 0x1003, 999)] {
        let mut spec = TitleSpec::add_on(APP, title, 0, 88);
        spec.required_application_version = required;
        let p = temp.native("bad.nsp", &spec);
        let request = NativeAddRequest::new(&p, title, 0);
        assert!(add_native_content(
            &current,
            temp.path("rejected.nx"),
            &request,
            &native_options(),
            &options,
            IntegrityPolicy::Strict
        )
        .is_err());
        let original = fs::read(&p).unwrap();
        assert!(add_native_content(
            &current,
            &p,
            &request,
            &native_options(),
            &options,
            IntegrityPolicy::Strict
        )
        .is_err());
        assert_eq!(fs::read(&p).unwrap(), original);
    }
}

#[test]
fn positional_reads_cross_blocks_and_leave_output_unchanged_on_failure() {
    let temp = Temp::new();
    let options = WriterOptions::default();
    let mut spec = TitleSpec::application(APP, 0, 1);
    spec.data = Some((0..700001).map(|i| (i % 251) as u8).collect());
    let p = temp.native("large.nsp", &spec);
    let inv = associate(vec![inspect_source(&p, &native_options())]);
    let nx = temp.path("large.nx");
    nxformat_packer::conversion::convert_inventory(
        &inv,
        &nx,
        &native_options(),
        IntegrityPolicy::Strict,
        &options,
    )
    .unwrap();
    let id = summary(&nx, &options).applications[0].snapshots[0].identity;
    let entries = title_entries(&spec);
    let (name, bytes) = entries.iter().max_by_key(|(_, b)| b.len()).unwrap();
    for offset in [0, 262143, 524287, bytes.len() - 77] {
        let end = (offset + 999).min(bytes.len());
        let mut result = vec![0; end - offset];
        read_snapshot_file_range(&nx, id, name, offset as u64, &mut result, &options).unwrap();
        assert_eq!(result, bytes[offset..end]);
    }
    let mut output = [5; 100];
    assert!(
        read_snapshot_file_range(&nx, id, name, bytes.len() as u64, &mut output, &options).is_err()
    );
    assert_eq!(output, [5; 100]);
    let cancelled = WriterOptions::default();
    cancelled.cancellation.cancel();
    assert!(read_snapshot_file_range(&nx, id, name, 0, &mut output, &cancelled).is_err());
    assert_eq!(output, [5; 100]);
}

#[test]
fn fast_in_place_addition_preserves_raw_frames_and_mixed_chunks_with_a_memory_ceiling() {
    let temp = Temp::new();
    let base_options = WriterOptions {
        chunk_size_bytes: 65536,
        compression_mode: CompressionMode::RawOnly,
        ..WriterOptions::default()
    };
    let nx = base(&temp, &base_options);
    let copy = temp.path("before.nx");
    fs::copy(&nx, &copy).unwrap();
    let mut spec = TitleSpec::patch(APP, 1, 10);
    spec.program.as_mut().unwrap().romfs_bytes = 400000;
    let p = temp.native("update.nsp", &spec);
    let options = WriterOptions {
        chunk_size_bytes: 1048576,
        max_memory_bytes: 128 * 1024 * 1024,
        ..WriterOptions::default()
    };
    let stats = add_native_content(
        &nx,
        &nx,
        &NativeAddRequest::new(p, APP | 0x800, 1),
        &native_options(),
        &options,
        IntegrityPolicy::Fast,
    )
    .unwrap();
    assert_eq!(stats.repack.source_payload_bytes_verified, 0);
    assert!(stats.accounted_memory_bytes <= options.max_memory_bytes);
    compare_retained_frames(&copy, &nx, &options);
    let mut expected = title_entries(&TitleSpec::application(APP, 0, 1));
    expected.extend(title_entries(&spec));
    compare_files(&nx, stats.added_snapshot_identity, &expected, &options);
    let chunks: std::collections::BTreeSet<_> = inspect_snapshot_files(
        &nx,
        stats.added_snapshot_identity,
        &options,
        IntegrityPolicy::Strict,
    )
    .unwrap()
    .iter()
    .map(|f| f.chunk_size_bytes)
    .collect();
    assert_eq!(chunks, [65536, 1048576].into_iter().collect());
}

#[test]
fn dlc_cannot_use_a_wire_valid_but_incomplete_retained_native_update() {
    let temp = Temp::new();
    let options = WriterOptions::default();
    let nx = base(&temp, &options);
    let id = summary(&nx, &options).applications[0].snapshots[0].identity;
    let spec = TitleSpec::patch(APP, 1, 10);
    let entries = title_entries(&spec);
    let meta = entries
        .iter()
        .find(|(name, _)| name.ends_with(".cnmt.nca"))
        .unwrap();
    let mut content = [0; 16];
    for (i, b) in content.iter_mut().enumerate() {
        *b = u8::from_str_radix(&meta.0[i * 2..i * 2 + 2], 16).unwrap();
    }
    let snapshot = SnapshotInput {
        addon_title_id: None,
        native: SnapshotNativeMetadata::default(),
        program_id: Some(APP),
        snapshot_type: SnapshotType::Update,
        numeric_version: 1,
        content_id: Some(content),
        build_id: Some([10; 32]),
        display_version: None,
        metadata: None,
        dependencies: vec![DependencyInput {
            kind: DependencyKind::RequiredBase,
            content_id: None,
            target_snapshot_identity: id,
        }],
        root: DirectoryInput::new(
            entries
                .into_iter()
                .map(|(name, b)| EntryInput::file(name, FileInput::from_bytes(b)))
                .collect(),
        ),
    };
    let mut plan = RepackPlan::new();
    plan.additions = ArchiveInput::new(vec![ApplicationInput {
        application_id: APP,
        platform: Platform::Switch1,
        programs: vec![ProgramInput {
            program_id: APP,
            kind: ProgramKind::Primary,
            content_id: None,
            npdm: None,
        }],
        snapshots: vec![snapshot],
    }]);
    let partial = temp.path("partial.nx");
    repack_archive(&nx, &partial, plan, &options, IntegrityPolicy::Strict).unwrap();
    // Container consistency/hashes do not establish a complete native view.
    summary(&partial, &options);
    let mut dlc = TitleSpec::add_on(APP, APP + 0x1001, 0, 77);
    dlc.required_application_version = 1;
    let p = temp.native("dlc.nsp", &dlc);
    let out = temp.path("out.nx");
    fs::write(&out, b"old").unwrap();
    assert!(add_native_content(
        &partial,
        &out,
        &NativeAddRequest::new(p, APP + 0x1001, 0),
        &native_options(),
        &options,
        IntegrityPolicy::Strict
    )
    .is_err());
    assert_eq!(fs::read(out).unwrap(), b"old");
}

#[test]
fn multi_program_versions_partial_executable_replacement_and_data_inheritance() {
    let temp = Temp::new();
    let options = WriterOptions::default();
    let mut base_spec = TitleSpec::application(APP, 0, 1);
    let mut other = base_spec.program.clone().unwrap();
    other.program_id = APP + 1;
    other.build_id = [7; 32];
    base_spec.extra_programs.push(other);
    let base_entries = title_entries(&base_spec);
    let p = temp.native("base.nsp", &base_spec);
    let nx = temp.path("base.nx");
    let inv = associate(vec![inspect_source(&p, &native_options())]);
    nxformat_packer::conversion::convert_inventory(
        &inv,
        &nx,
        &native_options(),
        IntegrityPolicy::Strict,
        &options,
    )
    .unwrap();
    fs::remove_file(p).unwrap();
    let original = summary(&nx, &options);
    assert_eq!(original.applications[0].programs.len(), 2);
    assert_eq!(original.applications[0].snapshots.len(), 2);
    let mut current = nx;
    for version in 1..=3 {
        let mut update = TitleSpec::patch(APP, version, 20 + version as u8);
        if version == 1 {
            update.program.as_mut().unwrap().program_id = APP + 1;
            update.control = None;
        }
        if version == 2 {
            let mut p = update.program.clone().unwrap();
            p.program_id = APP + 1;
            p.build_id = [99; 32];
            update.extra_programs.push(p);
        }
        if version == 3 {
            update.program = None;
            update.control = None;
            update.data = Some(vec![5; 300001]);
        }
        let entries = title_entries_custom(&update, if version == 1 { &[1] } else { &[] }, &[]);
        let p = temp.path("update.nsp");
        fs::write(&p, dnsp(&entries)).unwrap();
        let out = temp.path(&format!("v{version}.nx"));
        let stats = add_native_content(
            &current,
            &out,
            &NativeAddRequest::new(&p, APP | 0x800, version),
            &native_options(),
            &options,
            IntegrityPolicy::Strict,
        )
        .unwrap();
        assert_eq!(stats.added_snapshot_identities.len(), 2);
        let snapshots = summary(&out, &options).applications.remove(0).snapshots;
        let mut expected = base_entries.clone();
        expected.extend(entries);
        for pid in [APP, APP + 1] {
            let snapshot = snapshots
                .iter()
                .find(|s| s.numeric_version == version && s.program_id == Some(pid))
                .unwrap();
            let build = match (version, pid == APP) {
                (1, true) => 1,
                (1, false) => 21,
                (2, true) => 22,
                (2, false) => 99,
                (_, true) => 1,
                _ => 7,
            };
            assert_eq!(snapshot.build_id, Some([build; 32]));
            compare_files(&out, snapshot.identity, &expected, &options);
        }
        compare_retained_frames(&current, &out, &options);
        current = out;
    }
    let mut spec = TitleSpec::add_on(APP, APP + 0x1001, 0, 88);
    spec.required_application_version = 2;
    let p = temp.native("dlc.nsp", &spec);
    let out = temp.path("with-dlc.nx");
    add_native_content(
        &current,
        &out,
        &NativeAddRequest::new(p, APP + 0x1001, 0),
        &native_options(),
        &options,
        IntegrityPolicy::Strict,
    )
    .unwrap();
    assert_eq!(summary(&out, &options).applications[0].snapshots.len(), 9);
}

#[test]
fn base_patch_shared_program_and_control_require_exact_metadata_and_bytes() {
    let temp = Temp::new();
    let options = WriterOptions::default();
    let spec = TitleSpec::application(APP, 0, 1);
    let base_entries = title_entries(&spec);
    let p = temp.native("base.nsp", &spec);
    let mut patch = TitleSpec::patch(APP, 1, 10);
    patch.control = spec.control.clone();
    let program = base_entries.iter().find(|(_, b)| b[0x205] == 0).unwrap();
    let control = base_entries.iter().find(|(_, b)| b[0x205] == 2).unwrap();
    let entries = title_entries_custom(
        &patch,
        &[],
        &[
            (0, program.0.clone(), program.1.clone()),
            (1, control.0.clone(), control.1.clone()),
        ],
    );
    let update = temp.path("update.nsp");
    fs::write(&update, dnsp(&entries)).unwrap();
    let inv = associate(vec![
        inspect_source(&p, &native_options()),
        inspect_source(&update, &native_options()),
    ]);
    assert!(inv.plan().blocked.is_empty());
    let converted = temp.path("converted.nx");
    nxformat_packer::conversion::convert_inventory(
        &inv,
        &converted,
        &native_options(),
        IntegrityPolicy::Strict,
        &options,
    )
    .unwrap();
    let base_inv = associate(vec![inspect_source(&p, &native_options())]);
    let base = temp.path("base.nx");
    nxformat_packer::conversion::convert_inventory(
        &base_inv,
        &base,
        &native_options(),
        IntegrityPolicy::Strict,
        &options,
    )
    .unwrap();
    fs::remove_file(p).unwrap();
    let out = temp.path("added.nx");
    let request = NativeAddRequest::new(&update, APP | 0x800, 1);
    let stats = add_native_content(
        &base,
        &out,
        &request,
        &native_options(),
        &options,
        IntegrityPolicy::Strict,
    )
    .unwrap();
    assert_eq!(
        stats.new_native_bytes_staged,
        entries
            .iter()
            .find(|(n, _)| n.ends_with(".cnmt.nca"))
            .unwrap()
            .1
            .len() as u64
    );
    assert_eq!(summary(&converted, &options), summary(&out, &options));
    let mut conflicting = entries.clone();
    let unit = conflicting
        .iter_mut()
        .find(|(name, _)| name == &program.0)
        .unwrap();
    *unit.1.last_mut().unwrap() ^= 1;
    fs::write(&update, dnsp(&conflicting)).unwrap();
    let before = fs::read(&out).unwrap();
    assert!(add_native_content(
        &base,
        &out,
        &request,
        &native_options(),
        &options,
        IntegrityPolicy::Strict
    )
    .is_err());
    assert_eq!(fs::read(&out).unwrap(), before);
    let base_dump = temp.native("original.nsp", &spec);
    let inv = associate(vec![
        inspect_source(&base_dump, &native_options()),
        inspect_source(&update, &native_options()),
    ]);
    assert!(!inv.plan().blocked.is_empty());
}

#[test]
fn missing_corrupted_executables_and_unknown_program_offsets_fail_closed() {
    let temp = Temp::new();
    let options = WriterOptions::default();
    let nx = base(&temp, &options);
    let out = temp.path("old.nx");
    fs::write(&out, b"old").unwrap();
    for case in 0..4 {
        let mut spec = TitleSpec::patch(APP, 1, 10);
        if case == 3 {
            spec.program.as_mut().unwrap().build_id = [0; 32];
        }
        let mut entries = title_entries_custom(&spec, if case == 2 { &[7] } else { &[] }, &[]);
        if case < 2 {
            let program = entries.iter_mut().find(|(_, b)| b[0x205] == 0).unwrap();
            if case == 0 {
                let at = program.1.windows(5).position(|w| w == b"main\0").unwrap();
                program.1[at..at + 4].copy_from_slice(b"xxxx");
            } else {
                let at = program.1.windows(4).position(|w| w == b"NSO0").unwrap();
                program.1[at] = 0;
            }
        }
        let p = temp.path("bad.nsp");
        fs::write(&p, dnsp(&entries)).unwrap();
        assert!(add_native_content(
            &nx,
            &out,
            &NativeAddRequest::new(p, APP | 0x800, 1),
            &InventoryOptions::default(),
            &options,
            IntegrityPolicy::Strict
        )
        .is_err());
        assert_eq!(fs::read(&out).unwrap(), b"old");
    }
}

#[test]
fn noncontiguous_offsets_and_program_order_do_not_invent_identity_associations() {
    let temp = Temp::new();
    let options = WriterOptions::default();
    let mut spec = TitleSpec::application(APP, 0, 1);
    spec.program.as_mut().unwrap().program_id = APP + 8;
    let mut extra = spec.program.clone().unwrap();
    extra.program_id = APP + 1;
    extra.build_id = [7; 32];
    spec.extra_programs.push(extra);
    let p = temp.path("base.nsp");
    fs::write(&p, dnsp(&title_entries_custom(&spec, &[0, 7], &[]))).unwrap();
    let inv = associate(vec![inspect_source(&p, &native_options())]);
    let nx = temp.path("base.nx");
    nxformat_packer::conversion::convert_inventory(
        &inv,
        &nx,
        &native_options(),
        IntegrityPolicy::Strict,
        &options,
    )
    .unwrap();
    fs::remove_file(p).unwrap();
    let mut update = TitleSpec::patch(APP, 1, 55);
    update.program.as_mut().unwrap().program_id = APP + 1;
    update.control = None;
    let p = temp.path("update.nsp");
    fs::write(&p, dnsp(&title_entries_custom(&update, &[7], &[]))).unwrap();
    let out = temp.path("new.nx");
    let stats = add_native_content(
        &nx,
        &out,
        &NativeAddRequest::new(p, APP | 0x800, 1),
        &native_options(),
        &options,
        IntegrityPolicy::Strict,
    )
    .unwrap();
    let result = summary(&out, &options);
    let primary = result.applications[0]
        .snapshots
        .iter()
        .find(|s| s.identity == stats.added_snapshot_identity)
        .unwrap();
    assert_eq!(primary.program_id, Some(APP + 8));
    assert_eq!(primary.build_id, Some([1; 32]));
    assert_eq!(
        result.applications[0]
            .snapshots
            .iter()
            .find(|s| s.numeric_version == 1 && s.program_id == Some(APP + 1))
            .unwrap()
            .build_id,
        Some([55; 32])
    );
}

#[test]
fn distinct_control_ncas_follow_their_native_program_and_offset() {
    let temp = Temp::new();
    let options = WriterOptions::default();
    let mut base = TitleSpec::application(APP, 0, 1);
    base.control.as_mut().unwrap().title = "First game".into();
    let mut extra = base.program.clone().unwrap();
    extra.program_id = APP + 1;
    extra.build_id = [7; 32];
    base.extra_programs.push(extra);
    let mut second = TitleSpec::application(APP, 0, 55);
    second.program.as_mut().unwrap().program_id = APP + 1;
    second.control.as_mut().unwrap().title = "Second game".into();
    second.control.as_mut().unwrap().display_version = "1.second".into();
    let control = title_entries(&second)
        .into_iter()
        .find(|(_, b)| b[0x205] == 2)
        .unwrap();
    let entries = title_entries_custom(&base, &[0, 1, 0, 1], &[(3, control.0, control.1)]);
    let (control_name, mut bad_control) = entries
        .iter()
        .find(|(_, b)| {
            b[0x205] == 2 && u64::from_le_bytes(b[0x210..0x218].try_into().unwrap()) == APP + 1
        })
        .unwrap()
        .clone();
    bad_control[0x210..0x218].copy_from_slice(&APP.to_le_bytes());
    let section = u32::from_le_bytes(bad_control[0x240..0x244].try_into().unwrap()) as usize * 512;
    let romfs = u64::from_le_bytes(bad_control[0x430..0x438].try_into().unwrap()) as usize;
    bad_control[section + romfs] ^= 1; // Unreadable NACP must not hide owner/offset ambiguity.
    let ambiguous = temp.path("ambiguous-control.nsp");
    fs::write(
        &ambiguous,
        dnsp(&title_entries_custom(
            &base,
            &[0, 1, 0, 0],
            &[(3, control_name, bad_control)],
        )),
    )
    .unwrap();
    let fast = InventoryOptions {
        mode: InventoryMode::Fast,
        ..native_options()
    };
    let bad = associate(vec![inspect_source(&ambiguous, &fast)]);
    assert!(
        bad.sources[0]
            .issues
            .iter()
            .any(|i| i.message.contains("ambiguous Control")),
        "{:?}",
        bad.sources[0].issues
    );
    assert!(!bad.plan().blocked.is_empty());
    let p = temp.path("base.nsp");
    fs::write(&p, dnsp(&entries)).unwrap();
    let inv = associate(vec![inspect_source(&p, &native_options())]);
    assert_eq!(inv.sources[0].metas[0].meta.controls.len(), 2);
    let plan = inv.plan();
    assert!(plan.blocked.is_empty());
    for (pid, title, version) in [
        (APP, "First game", "1.0"),
        (APP + 1, "Second game", "1.second"),
    ] {
        let snapshot = plan.applications[0]
            .snapshots
            .iter()
            .find(|s| s.program_id == Some(pid))
            .unwrap();
        let input = snapshot.to_snapshot_input(None).unwrap();
        assert_eq!(input.metadata.unwrap().title.as_deref(), Some(title));
        assert_eq!(input.display_version.as_deref(), Some(version));
    }
    let nx = temp.path("base.nx");
    nxformat_packer::conversion::convert_inventory(
        &inv,
        &nx,
        &native_options(),
        IntegrityPolicy::Strict,
        &options,
    )
    .unwrap();
    let base_info = summary(&nx, &options);
    for native in &plan.applications[0].snapshots {
        let stored = base_info.applications[0]
            .snapshots
            .iter()
            .find(|s| s.program_id == native.program_id)
            .unwrap();
        assert_eq!(
            stored.nacp_digest,
            Some(native.control.as_ref().unwrap().nacp_digest)
        );
    }
    fs::remove_file(p).unwrap();
    let mut patch = TitleSpec::patch(APP, 1, 77);
    patch.program.as_mut().unwrap().program_id = APP + 1;
    patch.control.as_mut().unwrap().title = "Second updated".into();
    let p = temp.path("update.nsp");
    fs::write(&p, dnsp(&title_entries_custom(&patch, &[1, 1], &[]))).unwrap();
    let out = temp.path("added.nx");
    add_native_content(
        &nx,
        &out,
        &NativeAddRequest::new(p, APP | 0x800, 1),
        &native_options(),
        &options,
        IntegrityPolicy::Strict,
    )
    .unwrap();
    let result = summary(&out, &options);
    let first = result.applications[0]
        .snapshots
        .iter()
        .find(|s| s.program_id == Some(APP) && s.numeric_version == 1)
        .unwrap();
    let second = result.applications[0]
        .snapshots
        .iter()
        .find(|s| s.program_id == Some(APP + 1) && s.numeric_version == 1)
        .unwrap();
    assert!(
        first.nacp_digest.is_none(),
        "absent metadata must not be copied from another Program"
    );
    assert!(second.nacp_digest.is_some());
    assert_eq!(second.build_id, Some([77; 32]));
}

#[test]
fn converting_a_base_with_its_update_stores_the_base_once() {
    // The update's planned view also lists the base files; converting both
    // together must borrow them, like an in-place addition, not store them
    // twice. Compressed frames did not deduplicate, so use compressible data.
    let temp = Temp::new();
    let options = WriterOptions {
        ..WriterOptions::default()
    };
    let mut seed = 0x1234_5678_u32;
    let noise = (0..4_000_000)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 28) as u8
        })
        .collect::<Vec<u8>>();
    let mut base_spec = TitleSpec::application(APP, 0, 1);
    base_spec.data = Some(noise);
    let base_path = temp.native("base.nsp", &base_spec);
    let update_path = temp.native("update.nsp", &TitleSpec::patch(APP, 1, 10));

    let together = temp.path("together.nx");
    let inv = associate(vec![
        inspect_source(&base_path, &native_options()),
        inspect_source(&update_path, &native_options()),
    ]);
    nxformat_packer::conversion::convert_inventory(
        &inv,
        &together,
        &native_options(),
        IntegrityPolicy::Strict,
        &options,
    )
    .unwrap();

    let added = temp.path("added.nx");
    let inv = associate(vec![inspect_source(&base_path, &native_options())]);
    nxformat_packer::conversion::convert_inventory(
        &inv,
        &added,
        &native_options(),
        IntegrityPolicy::Strict,
        &options,
    )
    .unwrap();
    add_native_content(
        &added,
        &added,
        &NativeAddRequest::new(update_path, APP | 0x800, 1),
        &native_options(),
        &options,
        IntegrityPolicy::Strict,
    )
    .unwrap();

    assert_eq!(
        summary(&together, &options).applications[0].snapshots.len(),
        2
    );
    let together_size = fs::metadata(&together).unwrap().len();
    let added_size = fs::metadata(&added).unwrap().len();
    assert!(
        together_size <= added_size + added_size / 20,
        "base stored twice: {together_size} vs {added_size}"
    );
}

/// An update can bring a Program the base does not have: Bayonetta 3 v1.2
/// adds Program 1 (the Origins prologue). It used to be refused. Adding the
/// update to an NX and converting base + update together give the same
/// snapshots, and the new Program's tree is complete (base files + update).
#[test]
fn update_that_brings_a_new_program_is_accepted() {
    let temp = Temp::new();
    let options = WriterOptions::default();
    let base_spec = TitleSpec::application(APP, 0, 1);
    let base_source = temp.native("one-program-base.nsp", &base_spec);
    let mut update = TitleSpec::patch(APP, 1, 51);
    let mut extra = update.program.clone().unwrap();
    extra.program_id = APP + 1;
    extra.build_id = [9; 32];
    update.extra_programs.push(extra);
    let update_source = temp.native("two-program-update.nsp", &update);

    let added = base(&temp, &options);
    add_native_content(
        &added,
        &added,
        &NativeAddRequest::new(&update_source, APP | 0x800, 1),
        &native_options(),
        &options,
        IntegrityPolicy::Strict,
    )
    .unwrap();
    let snapshots = summary(&added, &options).applications.remove(0).snapshots;
    assert_eq!(snapshots.len(), 3);
    let mut expected = title_entries(&base_spec);
    expected.extend(title_entries(&update));
    for program in [APP, APP + 1] {
        let snapshot = snapshots
            .iter()
            .find(|s| s.snapshot_type == SnapshotType::Update && s.program_id == Some(program))
            .unwrap_or_else(|| panic!("no update snapshot for Program {program:016x}"));
        compare_files(&added, snapshot.identity, &expected, &options);
    }

    let converted = temp.path("converted.nx");
    let inventory = associate(vec![
        inspect_source(&base_source, &native_options()),
        inspect_source(&update_source, &native_options()),
    ]);
    nxformat_packer::conversion::convert_inventory(
        &inventory,
        &converted,
        &native_options(),
        IntegrityPolicy::Strict,
        &options,
    )
    .unwrap();
    let identities = |nx: &Path| {
        let mut ids: Vec<_> = summary(nx, &options).applications[0]
            .snapshots
            .iter()
            .map(|s| s.identity)
            .collect();
        ids.sort();
        ids
    };
    assert_eq!(identities(&converted), identities(&added));
}
