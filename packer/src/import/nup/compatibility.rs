//! Explicit, TOC-bound interpretation of nonconforming multi-program NUPs.
//! Mapping declarations are not NUP v1 conformance or signature authentication.
use super::*;
use crate::{Platform, ProgramKind, SnapshotType};

pub(super) struct Mapping {
    pub manifest: DeclarationManifest,
    pub indices: Vec<usize>,
    pub trees: Vec<Tree>,
}

fn small_file(
    path: &Path,
    f: &NupFile,
    token: &crate::CancellationToken,
    max: u64,
) -> Result<Vec<u8>, WriterError> {
    if f.size > max {
        return Err(limit("NUP compatibility metadata exceeds its size limit"));
    }
    let mut data = Vec::new();
    NupExtent {
        path: std::sync::Arc::new(path.to_owned()),
        name: f.name.clone(),
        offset: f.extent_offset,
        stored: f.extent_size,
        codec: f.codec,
        size: f.size,
        sha256: f.sha256,
    }
    .reader(token)?
    .read_to_end(&mut data)?;
    Ok(data)
}

/// Preserve noncanonical root content in an explicit, reversible namespace.
/// ExeFS/RomFS/data retain their meaning; extras never become game RomFS.
fn complete_tree(toc: &Toc, p: &Package, limits: &ImportLimits) -> Result<Tree, WriterError> {
    let mut raw = Tree::default();
    toc.walk(p.root, "", &mut raw, &mut BTreeSet::new(), 1, limits)?;
    for (index, expected) in [(p.exefs, "exefs"), (p.romfs, "romfs")] {
        if index != NONE {
            let d = toc
                .dirs
                .get(index as usize)
                .ok_or_else(|| bad("invalid content directory index"))?;
            let root = &toc.dirs[p.root as usize];
            let children = &toc.children
                [root.first_child as usize..(root.first_child + root.child_count) as usize];
            if d.name != expected || !children.contains(&index) {
                return Err(bad(
                    "mapped package ExeFS/RomFS must be named direct children",
                ));
            }
        } else if raw.dirs.contains(expected) {
            return Err(bad("unclaimed ExeFS/RomFS directory"));
        }
    }
    if raw.dirs.contains("data/__nup_root__")
        || raw.files.contains_key("data/__nup_root__")
        || raw.files.contains_key("data/__nup_mapping__.txt")
        || raw.dirs.contains("data/__nup_mapping__.txt")
    {
        return Err(bad(
            "reserved compatibility namespace data/__nup_root__ is occupied",
        ));
    }
    let translate = |p: &str| {
        if ["exefs", "romfs", "data"].contains(&p.split('/').next().unwrap()) {
            p.to_owned()
        } else {
            format!("data/__nup_root__/{p}")
        }
    };
    let mut tree = Tree::default();
    for d in raw.dirs {
        tree.dirs.insert(translate(&d));
    }
    for (p, i) in raw.files {
        tree.files.insert(translate(&p), i);
    }
    if tree
        .files
        .keys()
        .any(|p| p.starts_with("data/__nup_root__/"))
        || tree
            .dirs
            .iter()
            .any(|p| p.starts_with("data/__nup_root__/"))
    {
        tree.dirs.insert("data".into());
        tree.dirs.insert("data/__nup_root__".into());
    }
    Ok(tree)
}

impl Mapping {
    pub fn provenance(&self, toc: &Toc) -> String {
        let digest = toc
            .digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let mut text=format!("Explicit NUP compatibility interpretation; not NUP v1 conformance.\nTOC SHA256: {digest}\nCompleteness is user-declared; gameplay is untested.\nOriginal extra roots are preserved under data/__nup_root__/.\nOriginal package labels (index, title ID, kind, flags, version):\n");
        for (i, p) in toc.packages.iter().enumerate() {
            text.push_str(&format!(
                "{i}\t{:016x}\t{}\t{}\t{}\n",
                p.title_id, p.kind, p.flags, p.version
            ));
        }
        text.push_str("\nDeclarations:\n");
        text.push_str(std::str::from_utf8(self.manifest.bytes()).unwrap());
        text
    }
    pub fn read(
        path: &Path,
        nup: &Path,
        toc: &Toc,
        limits: &ImportLimits,
        token: &crate::CancellationToken,
    ) -> Result<Self, WriterError> {
        let file = File::open(path)?;
        let mut text = String::new();
        file.take(limits.max_manifest_bytes + 1)
            .read_to_string(&mut text)?;
        if text.len() as u64 > limits.max_manifest_bytes {
            return Err(limit("NUP mapping exceeds manifest budget"));
        }
        let (binding, body) = text
            .split_once('\n')
            .ok_or_else(|| bad("missing NXNUPMAP header"))?;
        let digest = toc
            .digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if binding.trim_end_matches('\r') != format!("NXNUPMAP\t1\t{digest}") {
            return Err(bad(
                "NUP mapping header/TOC hash mismatch; select the mapping for this exact container",
            ));
        }
        let manifest = DeclarationManifest::read(body.as_bytes(), limits)?;
        if manifest.snapshots.len() != toc.packages.len() {
            return Err(bad("mapping must describe every NUP package exactly once"));
        }
        let mut indices = Vec::new();
        let mut seen = BTreeSet::new();
        let mut scopes = BTreeSet::new();
        let mut trees = Vec::new();
        let mut total = 0;
        let first = &manifest.snapshots[0];
        let mut cnmts = Vec::new();
        let mut metadata_bytes = 0u64;
        for s in &manifest.snapshots {
            cancelled(token)?;
            portable_path(&format!("trees/{}", s.key), limits)?;
            let i = s
                .root
                .strip_prefix("package-")
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|i| *i < toc.packages.len())
                .ok_or_else(|| bad("mapping roots must be package-<zero-based index>"))?;
            if s.root != format!("package-{i}") || !seen.insert(i) {
                return Err(bad("duplicate/noncanonical mapped package index"));
            }
            if s.application_id != first.application_id || s.platform != first.platform {
                return Err(bad("mapping must declare one application and platform"));
            }
            if !s.complete
                || s.composition.is_some()
                || !s.exclusions.is_empty()
                || !s.presentation.is_empty()
                || s.build_id.is_some()
                || s.content_id.is_some()
            {
                return Err(bad("NUP mapping requires complete views without composition, exclusions or replacement identities"));
            }
            if !scopes.insert((
                s.program_id,
                s.addon_title_id,
                s.snapshot_type as u8,
                s.numeric_version,
            )) {
                return Err(bad("ambiguous duplicate program/version mapping"));
            }
            let p = &toc.packages[i];
            if p.kind > 2 || p.flags > 1 || (p.kind != 1 && p.flags != 0) {
                return Err(bad("invalid source package kind/flags"));
            }
            let tree = complete_tree(toc, p, limits)?;
            charge_entries(&mut total, &tree, MAX_RESOLVED_ENTRIES)?;
            if s.snapshot_type == SnapshotType::Dlc {
                if p.kind != 2
                    || s.addon_title_id != Some(p.title_id)
                    || s.numeric_version != p.version
                    || p.exefs != NONE
                    || s.program_id.is_some()
                    || s.program_kind != ProgramKind::Unknown
                {
                    return Err(bad("DLC mapping contradicts source package"));
                }
            } else {
                if p.kind == 2
                    || s.program_id.is_none()
                    || !matches!(
                        s.program_kind,
                        ProgramKind::Primary | ProgramKind::Auxiliary
                    )
                {
                    return Err(bad(
                        "executable package needs an explicit native Program ID and role",
                    ));
                }
                let index = tree
                    .files
                    .get("exefs/main.npdm")
                    .ok_or_else(|| bad("mapped executable package lacks main.npdm"))?;
                let bytes = small_file(nup, &toc.files[*index], token, 1024 * 1024)?;
                let facts = structures::validate_npdm(&bytes, s.npdm_profile)?;
                if Some(facts.program_id) != s.program_id {
                    return Err(bad(
                        "mapped Program ID differs from native NPDM; do not patch or relabel it",
                    ));
                }
            }
            for (name, index) in &tree.files {
                if name.starts_with("data/__nup_root__/meta/") && name.ends_with(".cnmt") {
                    metadata_bytes = metadata_bytes
                        .checked_add(toc.files[*index].size)
                        .filter(|n| *n <= limits.max_inventory_bytes)
                        .ok_or_else(|| limit("embedded CNMT metadata exceeds inventory budget"))?;
                    let bytes = small_file(nup, &toc.files[*index], token, 1024 * 1024)?;
                    let meta = crate::native::nca::parse_cnmt(&bytes)
                        .map_err(|e| bad(&format!("invalid embedded CNMT: {e}")))?;
                    if meta.application_id != s.application_id || meta.version != s.numeric_version
                    {
                        return Err(bad(
                            "package mapping contradicts its embedded CNMT application/version",
                        ));
                    }
                    let own_kind_matches = match meta.meta_type {
                        crate::native::ContentMetaType::Application => {
                            s.snapshot_type == SnapshotType::Base
                        }
                        crate::native::ContentMetaType::Patch => matches!(
                            s.snapshot_type,
                            SnapshotType::Update | SnapshotType::StandaloneUpdate
                        ),
                        crate::native::ContentMetaType::AddOnContent => {
                            s.snapshot_type == SnapshotType::Dlc
                                && s.addon_title_id == Some(meta.title_id)
                        }
                        _ => false,
                    };
                    if !own_kind_matches {
                        return Err(bad(
                            "package mapping contradicts its embedded CNMT kind/title",
                        ));
                    }
                    cnmts.push(meta);
                }
            }
            indices.push(i);
            trees.push(tree);
        }
        // One stable role per native Program ID; at most one primary program.
        let mut roles = BTreeMap::new();
        let mut primary = BTreeSet::new();
        for s in &manifest.snapshots {
            if let Some(id) = s.program_id {
                if roles
                    .insert(id, s.program_kind)
                    .is_some_and(|r| r != s.program_kind)
                {
                    return Err(bad("program role changes between versions"));
                }
                if s.program_kind == ProgramKind::Primary {
                    primary.insert(id);
                }
            }
            if s.snapshot_type == SnapshotType::Update {
                if s.dependencies.len() != 1 {
                    return Err(bad(
                        "mapped update needs one explicit matching-program base dependency",
                    ));
                }
                let (kind, key) = &s.dependencies[0];
                let b = manifest
                    .snapshots
                    .iter()
                    .find(|b| &b.key == key)
                    .ok_or_else(|| bad("missing mapped update base"))?;
                if *kind != crate::DependencyKind::RequiredBase
                    || b.snapshot_type != SnapshotType::Base
                    || b.program_id != s.program_id
                    || b.numeric_version >= s.numeric_version
                {
                    return Err(bad(
                        "update dependency is not an earlier base of the same program",
                    ));
                }
            } else if !s.dependencies.is_empty() {
                return Err(bad("unexpected mapping dependency"));
            }
        }
        if primary.len() != 1 {
            return Err(bad("mapping requires exactly one primary Program ID"));
        }
        for meta in &cnmts {
            let platform = match meta.platform_byte {
                0 => Platform::Switch1,
                1 => Platform::Switch2,
                _ => return Err(bad("unsupported embedded CNMT platform")),
            };
            if meta.application_id != first.application_id || platform != first.platform {
                return Err(bad(
                    "mapping contradicts embedded CNMT application/platform",
                ));
            }
            let relevant: Vec<_> = manifest
                .snapshots
                .iter()
                .filter(|s| {
                    if meta.meta_type == crate::native::ContentMetaType::AddOnContent {
                        s.addon_title_id == Some(meta.title_id)
                    } else {
                        s.program_id.is_some() && s.numeric_version == meta.version
                    }
                })
                .collect();
            if relevant.is_empty() {
                return Err(bad(
                    "mapping omits the version/title declared by embedded CNMT",
                ));
            }
            if matches!(
                meta.meta_type,
                crate::native::ContentMetaType::Application | crate::native::ContentMetaType::Patch
            ) {
                let offsets: BTreeSet<_> = meta
                    .contents
                    .iter()
                    .filter(|c| c.record_type == crate::native::ContentRecordType::Program)
                    .map(|c| c.id_offset)
                    .collect();
                if offsets.len()
                    != meta
                        .contents
                        .iter()
                        .filter(|c| c.record_type == crate::native::ContentRecordType::Program)
                        .count()
                {
                    return Err(bad("embedded CNMT repeats a Program content offset"));
                }
                if !offsets.is_empty() {
                    let mut declared = BTreeSet::new();
                    for d in &relevant {
                        let id = d
                            .program_id
                            .ok_or_else(|| bad("CNMT program mapping lacks native ID"))?;
                        // The generation prefix may differ (0100/0400); retain
                        // the full NPDM ID, checking the application family and
                        // the CNMT content index without synthesizing an ID.
                        if id & 0x00ff_ffff_ffff_ff00 != meta.application_id & 0x00ff_ffff_ffff_ff00
                            || !declared.insert(id as u8)
                        {
                            return Err(bad("mapped program is unrelated to the CNMT application or duplicates its content index"));
                        }
                    }
                    if declared != offsets {
                        return Err(bad(
                            "mapped program set differs from embedded CNMT Program content offsets",
                        ));
                    }
                }
            }
            for s in relevant {
                let compatible = match meta.meta_type {
                    crate::native::ContentMetaType::Application => {
                        s.snapshot_type == SnapshotType::Base
                    }
                    crate::native::ContentMetaType::Patch => matches!(
                        s.snapshot_type,
                        SnapshotType::Update | SnapshotType::StandaloneUpdate
                    ),
                    crate::native::ContentMetaType::AddOnContent => {
                        s.snapshot_type == SnapshotType::Dlc && s.numeric_version == meta.version
                    }
                    _ => false,
                };
                if !compatible {
                    return Err(bad(
                        "mapping snapshot kind/version contradicts embedded CNMT",
                    ));
                }
            }
        }
        Ok(Self {
            manifest,
            indices,
            trees,
        })
    }
}

pub(super) fn metadata(
    nup: &Path,
    toc: &Toc,
    tree: &Tree,
    d: &SnapshotDeclaration,
    mut meta: Meta,
    limits: &ImportLimits,
    token: &crate::CancellationToken,
) -> Result<Meta, WriterError> {
    let lookup = |p: &str| {
        tree.files
            .get(&format!("data/__nup_root__/{p}"))
            .ok_or_else(|| {
                bad("declared NUP control metadata is missing; use an original control/ path")
            })
    };
    if let Some(path) = &d.nacp {
        if !path.starts_with("control/") {
            return Err(bad("mapped NACP must select an original control/ file"));
        }
        let data = small_file(nup, &toc.files[*lookup(path)?], token, 0x4000)?;
        if data.len() != 0x4000 {
            return Err(bad("mapped control.nacp must be 16384 bytes"));
        }
        let (titles, version, _) = crate::native::parse_nacp_text(&data);
        if let Some(title) = titles.first() {
            meta.title = title.name.clone();
            meta.publisher = title.publisher.clone();
        }
        meta.display_version = version;
        meta.nacp = Some(data);
    }
    if !d.icons.is_empty() {
        meta.localized_icons.clear();
        meta.icon = None;
        for (language, path) in &d.icons {
            if !path.starts_with("control/") {
                return Err(bad("mapped icon must select an original control/ file"));
            }
            let tag = NACP_LANGUAGE_MAP
                .iter()
                .find(|(name, _)| name == language)
                .ok_or_else(|| bad("unknown mapped icon language"))?
                .1;
            let data = small_file(
                nup,
                &toc.files[*lookup(path)?],
                token,
                limits.max_image_bytes.min(16 * 1024 * 1024),
            )?;
            if meta.icon.is_none() {
                meta.icon = Some(data.clone());
            }
            meta.localized_icons.push((tag.into(), data));
        }
    }
    if let Some(nacp) = &meta.nacp {
        let (titles, version, _) = crate::native::parse_nacp_text(nacp);
        if let Some(t) = titles.first() {
            meta.title = t.name.clone().or(meta.title);
            meta.publisher = t.publisher.clone().or(meta.publisher);
        }
        meta.display_version = version.or(meta.display_version);
    }
    Ok(meta)
}

pub fn inspect_nup_with_mapping(
    path: &Path,
    mapping: &Path,
    limits: &ImportLimits,
    token: &crate::CancellationToken,
) -> Result<NupInfo, WriterError> {
    limits.validate()?;
    let mut file = File::open(path)?;
    let (toc, size) = read_toc(&mut file, limits)?;
    let map = Mapping::read(mapping, path, &toc, limits, token)?;
    let first = &map.manifest.snapshots[0];
    let mut info = NupInfo {
        application_id: first.application_id,
        platform: first.platform,
        size,
        title: None,
        publisher: None,
        icon: None,
        packages: Vec::new(),
    };
    let mut used = 0u64;
    for (j, s) in map.manifest.snapshots.iter().enumerate() {
        used = used.saturating_add(u64::from(toc.packages[map.indices[j]].meta_size));
        for path in s.nacp.iter().chain(s.icons.values()) {
            if let Some(index) = map.trees[j].files.get(&format!("data/__nup_root__/{path}")) {
                used = used.saturating_add(toc.files[*index].size);
            }
        }
        if used > limits.max_inventory_bytes {
            return Err(limit("mapped display metadata exceeds inventory budget"));
        }
        let meta = metadata(
            path,
            &toc,
            &map.trees[j],
            s,
            read_meta(&mut file, &toc.packages[map.indices[j]], size)?,
            limits,
            token,
        )?;
        if s.program_kind == ProgramKind::Primary {
            info.title = meta.title;
            info.publisher = meta.publisher;
            info.icon = meta.icon;
        }
        info.packages.push(NupPackageInfo {
            kind: s.snapshot_type,
            version: s.numeric_version,
            display_version: meta.display_version,
        });
    }
    Ok(info)
}

#[cfg(test)]
#[path = "compatibility_tests.rs"]
mod tests;
