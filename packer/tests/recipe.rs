//! Byte-exact rebuild recipes on synthetic DNSP/DXCI packages.

use nxformat_packer::conversion::convert_inventory;
use nxformat_packer::native::synthetic::{dnsp, dxci, title_entries, TitleSpec};
use nxformat_packer::native::{inventory, InventoryMode, InventoryOptions};
use nxformat_packer::recipe::{
    check_recipe, create_recipe, rebuild_from_recipe, recipe_source_name,
};
use nxformat_packer::{
    inspect_archive, repack_archive, ArchiveInput, IntegrityPolicy, RepackPlan, SnapshotType,
    WriterOptions,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const APP: u64 = 0x0100_0000_0001_0000;

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("nx-recipe-test-{}-{id}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.0.join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, bytes).unwrap();
        p
    }
    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn strict() -> InventoryOptions {
    InventoryOptions {
        mode: InventoryMode::Strict,
        ..InventoryOptions::default()
    }
}

fn convert(sources: &[PathBuf], out: &Path) {
    let inv = inventory(sources, &strict()).unwrap();
    convert_inventory(
        &inv,
        out,
        &strict(),
        IntegrityPolicy::Strict,
        &WriterOptions::default(),
    )
    .unwrap();
}

/// A ticket and a certificate are not stored in the NX; the recipe keeps them.
fn with_ticket(mut entries: Vec<(String, Vec<u8>)>, seed: u8) -> Vec<(String, Vec<u8>)> {
    entries.push((
        format!("{:032x}.tik", APP as u128 + seed as u128),
        (0..0x2c0u32)
            .map(|i| (i as u8).wrapping_mul(seed))
            .collect(),
    ));
    entries.push((
        format!("{:032x}.cert", APP as u128 + seed as u128),
        vec![seed; 0x700],
    ));
    entries
}

#[test]
fn dnsp_rebuilds_byte_for_byte() {
    let temp = TempDir::new();
    let source = temp.write(
        "src/Game [base].nsp",
        &dnsp(&with_ticket(
            title_entries(&TitleSpec::application(APP, 0, 1)),
            3,
        )),
    );
    let nx = temp.join("game.nx");
    convert(std::slice::from_ref(&source), &nx);
    let recipe = temp.join("game.nxrecipe");
    let options = WriterOptions::default();
    let stats = create_recipe(&source, &nx, &recipe, &options).unwrap();
    assert!(stats.referenced_bytes > 0);
    assert_eq!(
        stats.unmatched_entries.len(),
        2,
        "{:?}",
        stats.unmatched_entries
    );
    assert!(
        stats.recipe_bytes < 4096,
        "recipe is {} bytes",
        stats.recipe_bytes
    );
    assert_eq!(recipe_source_name(&recipe).unwrap(), "Game [base].nsp");
    let check = check_recipe(&nx, &recipe, &options).unwrap();
    assert_eq!(check.bytes, fs::metadata(&source).unwrap().len());
    let rebuilt = temp.join("out/rebuilt.nsp");
    fs::create_dir(temp.join("out")).unwrap();
    rebuild_from_recipe(&nx, &recipe, &rebuilt, &options).unwrap();
    assert_eq!(fs::read(&rebuilt).unwrap(), fs::read(&source).unwrap());
    // Never overwrites.
    assert!(rebuild_from_recipe(&nx, &recipe, &rebuilt, &options).is_err());
}

#[test]
fn dxci_padding_becomes_fill_and_rebuilds() {
    let temp = TempDir::new();
    let mut image = dxci(&[
        ("update", vec![("firmware.bin".to_owned(), vec![7u8; 5000])]),
        ("normal", Vec::new()),
        ("secure", title_entries(&TitleSpec::application(APP, 0, 4))),
    ]);
    // Untrimmed card: 0xFF up to the card size.
    image.resize(image.len() + 3 * 1024 * 1024 + 123, 0xff);
    let source = temp.write("src/game.xci", &image);
    let nx = temp.join("game.nx");
    convert(std::slice::from_ref(&source), &nx);
    let recipe = temp.join("game.nxrecipe");
    let options = WriterOptions::default();
    let stats = create_recipe(&source, &nx, &recipe, &options).unwrap();
    assert!(stats.fill_bytes >= 3 * 1024 * 1024, "{stats:?}");
    assert!(
        stats.recipe_bytes < 8192,
        "recipe is {} bytes",
        stats.recipe_bytes
    );
    // The system update partition is not inventoried: its bytes stay literal.
    assert!(stats.literal_bytes >= 5000, "{stats:?}");
    let rebuilt = temp.join("rebuilt.xci");
    rebuild_from_recipe(&nx, &recipe, &rebuilt, &options).unwrap();
    assert_eq!(fs::read(&rebuilt).unwrap(), image);
}

#[test]
fn several_titles_in_one_package_and_edits() {
    let temp = TempDir::new();
    let mut entries = title_entries(&TitleSpec::application(APP, 0, 1));
    entries.extend(title_entries(&TitleSpec::patch(APP, 65536, 2)));
    let source = temp.write("src/all.nsp", &dnsp(&entries));
    let nx = temp.join("game.nx");
    convert(std::slice::from_ref(&source), &nx);
    let recipe = temp.join("all.nxrecipe");
    let options = WriterOptions::default();
    create_recipe(&source, &nx, &recipe, &options).unwrap();
    check_recipe(&nx, &recipe, &options).unwrap();
    // Removing the update removes files the recipe needs: rebuilding fails
    // cleanly and leaves nothing behind.
    let summary = inspect_archive(&nx, &options, IntegrityPolicy::Fast).unwrap();
    let update = summary.applications[0]
        .snapshots
        .iter()
        .find(|s| s.snapshot_type == SnapshotType::Update)
        .unwrap()
        .identity;
    repack_archive(
        &nx,
        &nx,
        RepackPlan {
            remove_snapshots: vec![update],
            additions: ArchiveInput::new(Vec::new()),
            metadata_sources: Vec::new(),
        },
        &options,
        IntegrityPolicy::Strict,
    )
    .unwrap();
    let err = check_recipe(&nx, &recipe, &options).unwrap_err();
    assert!(
        err.to_string().contains("no longer in the archive"),
        "{err}"
    );
    let rebuilt = temp.join("rebuilt.nsp");
    assert!(rebuild_from_recipe(&nx, &recipe, &rebuilt, &options).is_err());
    assert!(!rebuilt.exists());
}

#[test]
fn damaged_recipe_is_refused() {
    let temp = TempDir::new();
    let source = temp.write(
        "src/game.nsp",
        &dnsp(&title_entries(&TitleSpec::application(APP, 0, 5))),
    );
    let nx = temp.join("game.nx");
    convert(std::slice::from_ref(&source), &nx);
    let recipe = temp.join("game.nxrecipe");
    let options = WriterOptions::default();
    create_recipe(&source, &nx, &recipe, &options).unwrap();
    let good = fs::read(&recipe).unwrap();
    for at in [0, 20, good.len() / 2, good.len() - 1] {
        let mut bad = good.clone();
        bad[at] ^= 1;
        fs::write(&recipe, &bad).unwrap();
        assert!(check_recipe(&nx, &recipe, &options).is_err(), "byte {at}");
    }
    fs::write(&recipe, &good[..good.len() - 10]).unwrap();
    assert!(check_recipe(&nx, &recipe, &options).is_err());
    // A source with nothing in the archive gets no recipe.
    let other = temp.write(
        "src/other.nsp",
        &dnsp(&title_entries(&TitleSpec::application(APP + 0x2000, 0, 9))),
    );
    assert!(create_recipe(&other, &nx, &temp.join("x.nxrecipe"), &options).is_err());
}

#[test]
fn recipes_kept_inside_the_nx_follow_edits() {
    use nxformat_packer::recipe::{
        build_recipe, check_embedded, embed_recipes, embedded_recipes, rebuild_embedded,
        remove_recipes,
    };
    let temp = TempDir::new();
    let base = temp.write(
        "src/Game [base].nsp",
        &dnsp(&with_ticket(
            title_entries(&TitleSpec::application(APP, 0, 1)),
            3,
        )),
    );
    let update = temp.write(
        "src/Game [update].nsp",
        &dnsp(&with_ticket(title_entries(&TitleSpec::patch(APP, 1, 9)), 4)),
    );
    let nx = temp.join("game.nx");
    convert(&[base.clone(), update.clone()], &nx);
    let options = WriterOptions::default();
    let recipes: Vec<Vec<u8>> = [&base, &update]
        .iter()
        .map(|s| build_recipe(s, &nx, &options).unwrap().0)
        .collect();
    embed_recipes(&nx, recipes, &options).unwrap();
    let kept = embedded_recipes(&nx, &options).unwrap();
    assert_eq!(kept.len(), 2);
    assert!(kept.iter().all(|r| r.application_id == APP));
    // The C-compatible readers skip the optional extension; strict checks pass.
    inspect_archive(&nx, &options, IntegrityPolicy::Strict).unwrap();
    fs::create_dir(temp.join("out")).unwrap();
    for r in &kept {
        let out = temp.join("out").join(&r.info.name);
        rebuild_embedded(&nx, &r.bytes, &out, &options).unwrap();
        let original = if r.info.name.contains("update") {
            &update
        } else {
            &base
        };
        assert_eq!(fs::read(&out).unwrap(), fs::read(original).unwrap());
        assert!(rebuild_embedded(&nx, &r.bytes, &out, &options).is_err());
    }
    // Embedding the same recipe again changes nothing.
    embed_recipes(&nx, vec![kept[0].bytes.clone()], &options).unwrap();
    assert_eq!(embedded_recipes(&nx, &options).unwrap().len(), 2);
    // Removing the update drops the recipe that needs its files.
    let summary = inspect_archive(&nx, &options, IntegrityPolicy::Fast).unwrap();
    let update_id = summary.applications[0]
        .snapshots
        .iter()
        .find(|s| s.snapshot_type == SnapshotType::Update)
        .unwrap()
        .identity;
    let mut plan = RepackPlan::new();
    plan.remove_snapshots.push(update_id);
    repack_archive(&nx, &nx, plan, &options, IntegrityPolicy::Strict).unwrap();
    let left = embedded_recipes(&nx, &options).unwrap();
    assert_eq!(left.len(), 1);
    assert!(left[0].info.name.contains("base"));
    check_embedded(&nx, &left[0].bytes, &options).unwrap();
    // A recipe can also be dropped on its own.
    remove_recipes(&nx, vec![left[0].info.recipe_sha256], &options).unwrap();
    assert!(embedded_recipes(&nx, &options).unwrap().is_empty());
    // A recipe naming a file that is not in the archive is refused.
    let foreign = temp.write(
        "src/Other.nsp",
        &dnsp(&title_entries(&TitleSpec::application(APP + 0x2000, 0, 7))),
    );
    let other_nx = temp.join("other.nx");
    convert(std::slice::from_ref(&foreign), &other_nx);
    let other = build_recipe(&foreign, &other_nx, &options).unwrap().0;
    assert!(embed_recipes(&nx, vec![other], &options).is_err());
}
