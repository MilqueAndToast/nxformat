use super::*;
use crate::native::{ContentMetaType as Kind, ContentRecordType};

/// Generate a checked compatibility declaration from embedded NPDM/CNMT.
/// The caller must explicitly accept that stored program views are complete.
/// Metadata cannot prove materialization completeness or runtime behavior.
pub fn generate_nup_mapping(
    nup: &Path,
    limits: &ImportLimits,
    token: &crate::CancellationToken,
) -> Result<String, WriterError> {
    limits.validate()?;
    cancelled(token)?;
    let (toc, _) = read_toc(&mut File::open(nup)?, limits)?;
    if toc.packages.len() > limits.max_snapshots {
        return Err(limit("NUP snapshot budget"));
    }
    let mut trees = Vec::new();
    let mut metas = Vec::new();
    let mut programs = Vec::new();
    let mut used = 0u64;
    let mut entries = 0;
    let mut unsupported = Vec::new();
    for (i, p) in toc.packages.iter().enumerate() {
        cancelled(token)?;
        let tree = complete_tree(&toc, p, limits)?;
        charge_entries(&mut entries, &tree, MAX_RESOLVED_ENTRIES)?;
        let program = if let Some(index) = tree.files.get("exefs/main.npdm") {
            let bytes = small_file(nup, &toc.files[*index], token, 1024 * 1024)?;
            let profile = npdm_profile(&bytes);
            let kind = match profile {
                "meta-acid2048-v1" => structures::NpdmProfile::MetaAcid2048V1,
                "meta-acid3072-v1" => structures::NpdmProfile::MetaAcid3072V1,
                _ => return Err(bad("cannot generate mapping: unsupported NPDM profile")),
            };
            Some((structures::validate_npdm(&bytes, kind)?.program_id, profile))
        } else {
            None
        };
        for (name, index) in &tree.files {
            if name.starts_with("data/__nup_root__/meta/") && name.ends_with(".cnmt") {
                used = used
                    .checked_add(toc.files[*index].size)
                    .filter(|n| *n <= limits.max_inventory_bytes)
                    .ok_or_else(|| limit("CNMT inventory budget"))?;
                let bytes = small_file(nup, &toc.files[*index], token, 1024 * 1024)?;
                let meta = crate::native::nca::parse_cnmt(&bytes)
                    .map_err(|e| bad(&format!("invalid embedded CNMT: {e}")))?;
                if !matches!(
                    meta.meta_type,
                    Kind::Application | Kind::Patch | Kind::AddOnContent
                ) {
                    unsupported.push(format!("package-{i} contains {:?} metadata; this importer requires complete program and AddOnContent views, and cannot compose DLC data patches", meta.meta_type));
                }
                metas.push((i, meta));
            }
        }
        programs.push(program);
        trees.push(tree);
    }
    if metas.is_empty() {
        return Err(bad(
            "cannot generate mapping: no embedded meta/*.cnmt; use a reviewed manual mapping",
        ));
    }
    if !programs.iter().flatten().any(|(id, _)| *id as u8 == 0) {
        unsupported.push("No primary executable (native program index 0) is present in the package table; an auxiliary program cannot be relabelled as the main game".into());
    }
    if !unsupported.is_empty() {
        return Err(bad(&format!("Cannot generate mapping: {}. Re-export complete program/DLC views with their original metadata, including the main executable; retry generation on that export.", unsupported.join(". "))));
    }
    let executable_metas: Vec<_> = metas
        .iter()
        .filter(|(_, m)| matches!(m.meta_type, Kind::Application | Kind::Patch))
        .collect();
    let (_, first) = executable_metas.first().ok_or_else(|| bad("Cannot generate mapping: no executable Application/Patch CNMT. Re-export the program views with their original meta/ metadata, or use a reviewed manual mapping."))?;
    let app = first.application_id;
    let marker = first.platform_byte;
    if executable_metas
        .iter()
        .any(|(_, m)| m.application_id != app || m.platform_byte != marker)
    {
        let pairs: BTreeSet<_> = executable_metas
            .iter()
            .map(|(_, m)| (m.application_id, m.platform_byte))
            .collect();
        let details = pairs
            .iter()
            .take(8)
            .map(|(a, p)| format!("{a:016x}/platform-{p}"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(bad(&format!("cannot generate mapping: conflicting executable CNMT applications/platforms ({details}). Re-export a coherent program set with its original metadata; a mapping cannot override executable identity conflicts")));
    }
    let platform = match marker {
        0 => "switch1",
        1 => "switch2",
        _ => return Err(bad("unsupported CNMT platform")),
    };
    for (i, meta) in &metas {
        if meta.application_id != app
            || !cnmt_platform_matches(
                meta,
                if marker == 1 {
                    Platform::Switch2
                } else {
                    Platform::Switch1
                },
                marker == 1,
            )
        {
            return Err(bad(&format!("package-{i}: DLC metadata belongs to a different application or incompatible platform. Re-export matching content for application {app:016x}; do not edit its IDs or platform markers")));
        }
    }
    let mut selected = Vec::new();
    for (i, p) in toc.packages.iter().enumerate() {
        let matching = |m: &crate::native::ContentMeta| match programs[i] {
            Some((id, _)) => {
                p.kind != 2
                    && matches!(m.meta_type, Kind::Application | Kind::Patch)
                    && id & 0x00ff_ffff_ffff_ff00 == app & 0x00ff_ffff_ffff_ff00
                    && m.contents.iter().any(|c| {
                        c.record_type == ContentRecordType::Program && c.id_offset == id as u8
                    })
            }
            None => {
                p.kind == 2
                    && m.meta_type == Kind::AddOnContent
                    && m.title_id == p.title_id
                    && m.version == p.version
            }
        };
        // Local CNMT is stronger evidence than a matching CNMT in a sibling.
        // Without it, a unique global relationship is required; never pick by order.
        let own: Vec<_> = metas.iter().filter(|(owner, _)| *owner == i).collect();
        let candidates: Vec<_> = if own.is_empty() {
            metas.iter().filter(|(_, m)| matching(m)).collect()
        } else {
            if own.iter().any(|(_, m)| !matching(m)) {
                return Err(bad(&format!(
                    "package-{i}: CNMT does not match its native program/DLC"
                )));
            }
            own
        };
        let identities: BTreeSet<_> = candidates
            .iter()
            .map(|(_, m)| {
                (
                    m.title_id,
                    m.version,
                    match m.meta_type {
                        Kind::Application => 0,
                        Kind::Patch => 1,
                        _ => 2,
                    },
                )
            })
            .collect();
        if identities.len() != 1 {
            return Err(bad(&format!("package-{i}: missing or ambiguous CNMT relationship; use a reviewed manual mapping")));
        }
        selected.push(&candidates[0].1);
    }
    let digest = toc
        .digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let mut text = format!("NXNUPMAP\t1\t{digest}\nNXIMPORT\t1\n");
    for (i, meta) in selected.iter().enumerate() {
        let key = format!("pkg-{i}");
        if let Some((id, profile)) = programs[i] {
            // Native index zero is the documented generation convention, not
            // a conclusion from package order, artwork or generation prefix.
            let role = if id as u8 == 0 {
                "primary"
            } else {
                "auxiliary"
            };
            let bases: Vec<_> = selected
                .iter()
                .enumerate()
                .filter(|(j, b)| {
                    b.meta_type == Kind::Application
                        && programs[*j].map(|p| p.0) == Some(id)
                        && b.version < meta.version
                })
                .collect();
            let kind = match meta.meta_type {
                Kind::Application => "base",
                Kind::Patch if bases.len() == 1 => "update",
                Kind::Patch if bases.is_empty() => "standalone-update",
                _ => {
                    return Err(bad(
                        "cannot generate mapping: ambiguous matching-program base",
                    ))
                }
            };
            text.push_str(&format!("snapshot\t{key}\tpackage-{i}\t{platform}\t{app:016x}\t{id:016x}\t{role}\t{kind}\t{}\tcomplete\tmaterialized-v1\t{profile}\n", meta.version));
            if kind == "update" {
                text.push_str(&format!(
                    "dependency\t{key}\trequired-base\tpkg-{}\n",
                    bases[0].0
                ));
            }
            // Only select an unambiguous control file. All other controls remain
            // preserved as extras; never guess between editions by directory name.
            let controls: Vec<_> = trees[i]
                .files
                .keys()
                .filter(|p| {
                    p.starts_with("data/__nup_root__/control/") && p.ends_with("/control.nacp")
                })
                .collect();
            if controls.len() == 1 {
                let path = controls[0].strip_prefix("data/__nup_root__/").unwrap();
                text.push_str(&format!("nacp\t{key}\t{path}\n"));
                let parent = path.rsplit_once('/').unwrap().0;
                for (language, _) in NACP_LANGUAGE_MAP {
                    let icon = format!("{parent}/icon_{language}.dat");
                    if trees[i]
                        .files
                        .contains_key(&format!("data/__nup_root__/{icon}"))
                    {
                        text.push_str(&format!("icon\t{key}\t{language}\t{icon}\n"));
                    }
                }
            }
        } else {
            text.push_str(&format!("snapshot\t{key}\tpackage-{i}\t{platform}\t{app:016x}\t-\tnone\tdlc\t{}\tcomplete\tmaterialized-v1\tnone\naddon\t{key}\t{:016x}\n",meta.version,meta.title_id));
        }
    }
    // Reuse the exact import validator before publishing any generated bytes.
    let mapping = Mapping::parse(&text, nup, &toc, limits, token)?;
    let mut source = File::open(nup)?;
    let physical = source.metadata()?.len();
    let mut display_bytes = 0u64;
    for (j, declaration) in mapping.manifest.snapshots.iter().enumerate() {
        let package = &toc.packages[mapping.indices[j]];
        display_bytes = display_bytes.saturating_add(u64::from(package.meta_size));
        for path in declaration.nacp.iter().chain(declaration.icons.values()) {
            if let Some(index) = mapping.trees[j]
                .files
                .get(&format!("data/__nup_root__/{path}"))
            {
                display_bytes = display_bytes.saturating_add(toc.files[*index].size);
            }
        }
        if display_bytes > limits.max_inventory_bytes {
            return Err(limit("mapped display metadata exceeds inventory budget"));
        }
        metadata(
            nup,
            &toc,
            &mapping.trees[j],
            declaration,
            read_meta(&mut source, package, physical)?,
            limits,
            token,
        )?;
    }
    cancelled(token)?;
    Ok(text)
}
