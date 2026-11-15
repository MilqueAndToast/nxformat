use super::*;

/// Narrow fallback for one complete native executable selected by native NACP.
/// Launch selection comes from the producer-selected NACP and an identical
/// original control file, not package order, artwork, names or ID prefixes.
pub(super) fn generate_from_control(
    nup: &Path,
    toc: &Toc,
    trees: &[Tree],
    programs: &[Option<(u64, &'static str)>],
    metas: &[(usize, crate::native::ContentMeta)],
    limits: &ImportLimits,
    token: &crate::CancellationToken,
) -> Result<String, WriterError> {
    let missing = || {
        bad("Automatic launch-role inference is unavailable: no executable CNMT and no unambiguous validated selected NACP. This does not prove executable data is missing; use a reviewed explicit mapping or retain original launch metadata.")
    };
    let executables: Vec<_> = programs
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.map(|p| (i, p)))
        .collect();
    if executables.len() != 1 {
        return Err(missing());
    }
    let (launch, (id, profile)) = executables[0];
    let app = metas[0].1.application_id;
    let marker = if metas.iter().any(|(_, m)| m.platform_byte == 1) {
        1
    } else {
        0
    };
    let platform = if marker == 1 {
        Platform::Switch2
    } else {
        Platform::Switch1
    };
    if metas
        .iter()
        .any(|(_, m)| m.application_id != app || !cnmt_platform_matches(m, platform, marker == 1))
        || id & 0x00ff_ffff_ffff_ff00 != app & 0x00ff_ffff_ffff_ff00
    {
        return Err(missing());
    }
    let mut file = File::open(nup)?;
    let physical = file.metadata()?.len();
    let selected = read_meta(&mut file, &toc.packages[launch], physical)?
        .nacp
        .ok_or_else(missing)?;
    let index = id as u8;
    if selected.len() == 0x4000 && u32_at(&selected, 0x34d4) == 0 && u64_at(&selected, 0x3070) == 0
    {
        return Err(missing());
    }
    if selected.len() != 0x4000
        || index >= 32
        || selected[0x3212] != index
        || selected[0x34d1] != index
        || selected[0x3217] != marker
        || u32_at(&selected, 0x34d4) & (1u32 << index) == 0
        || u64_at(&selected, 0x3070) != app.checked_add(0x1000).ok_or_else(missing)?
    {
        return Err(bad("Selected NACP launch index/accessibility/platform/application contradicts the actual native NPDM program; use original matching metadata"));
    }
    let acd = selected[0x3216];
    let expected = if acd == 0 {
        "control/control.nacp".into()
    } else {
        format!("control/Acd_{acd}/control.nacp")
    };
    let stored = format!("data/__nup_root__/{expected}");
    let source_index = trees[launch].files.get(&stored).ok_or_else(missing)?;
    if small_file(nup, &toc.files[*source_index], token, 0x4000)? != selected {
        return Err(bad("Selected NACP TLV differs from its original control file; launch metadata is conflicting"));
    }
    let p = &toc.packages[launch];
    let native: Vec<_> = metas
        .iter()
        .filter(|(_, m)| matches!(m.meta_type, Kind::Application | Kind::Patch))
        .collect();
    let authoritative = if native.is_empty() {
        None
    } else {
        let identities: BTreeSet<_> = native
            .iter()
            .map(|(_, m)| (m.title_id, m.version, m.meta_type == Kind::Patch))
            .collect();
        if identities.len() != 1
            || native.iter().any(|(_, m)| {
                !m.contents
                    .iter()
                    .any(|c| c.record_type == ContentRecordType::Program && c.id_offset == index)
            })
        {
            return Err(bad("Executable CNMT does not unambiguously identify the selected native program/version"));
        }
        Some(&native[0].1)
    };
    let version = authoritative.map_or(p.version, |m| m.version);
    let kind = match authoritative.map(|m| m.meta_type) {
        Some(Kind::Application) => "base",
        Some(Kind::Patch) => "standalone-update",
        None => match (p.kind, p.flags) {
            (0, 0) => "base",
            (1, 1) => "standalone-update",
            _ => {
                return Err(bad(
                    "NACP launch inference requires a complete base or standalone program view",
                ))
            }
        },
        _ => unreachable!(),
    };
    let platform = if marker == 1 { "switch2" } else { "switch1" };
    let digest = toc
        .digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let mut text=format!("NXNUPMAP\t1\t{digest}\nNXIMPORT\t1\nsnapshot\tpkg-{launch}\tpackage-{launch}\t{platform}\t{app:016x}\t{id:016x}\tprimary\t{kind}\t{version}\tcomplete\tmaterialized-v1\t{profile}\nnacp\tpkg-{launch}\t{expected}\n");
    let parent = expected.rsplit_once('/').unwrap().0;
    for (language, _) in NACP_LANGUAGE_MAP {
        let icon = format!("{parent}/icon_{language}.dat");
        if trees[launch]
            .files
            .contains_key(&format!("data/__nup_root__/{icon}"))
        {
            text.push_str(&format!("icon\tpkg-{launch}\t{language}\t{icon}\n"));
        }
    }
    for (i, p) in toc
        .packages
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != launch)
    {
        let matches = |m: &crate::native::ContentMeta| {
            p.kind == 2
                && ((m.meta_type == Kind::AddOnContent
                    && m.title_id == p.title_id
                    && m.version == p.version)
                    || (m.meta_type == Kind::DataPatch && m.data_id == Some(p.title_id)))
        };
        let own: Vec<_> = metas.iter().filter(|(owner, _)| *owner == i).collect();
        let candidates: Vec<_> = if own.is_empty() {
            metas.iter().filter(|(_, m)| matches(m)).collect()
        } else {
            if own.iter().any(|(_, m)| !matches(m)) {
                return Err(bad("DLC mapping contradicts its own native CNMT"));
            }
            own
        };
        let identities: BTreeSet<_> = candidates
            .iter()
            .map(|(_, m)| (m.title_id, m.version, m.meta_type == Kind::DataPatch))
            .collect();
        if identities.len() != 1 {
            return Err(bad("Missing or ambiguous native DLC relationship"));
        }
        let meta = &candidates[0].1;
        text.push_str(&format!("snapshot\tpkg-{i}\tpackage-{i}\t{platform}\t{app:016x}\t-\tnone\tdlc\t{}\tcomplete\tmaterialized-v1\tnone\naddon\tpkg-{i}\t{:016x}\n",meta.version,meta.data_id.unwrap_or(meta.title_id)));
        if let Some(known) = authoritative {
            let minimum = meta.required_application_version.unwrap_or(0);
            if known.version < minimum {
                return Err(bad(&format!("DLC {:016x} requires application version {minimum}, but authoritative executable CNMT establishes version {}",meta.title_id,known.version)));
            }
            if minimum > 0 {
                text.push_str(&format!("dependency\tpkg-{i}\trequires\tpkg-{launch}\n"));
            }
        }
        // No exact requires identity can establish a native minimum while the
        // application's authoritative version is unknown. Mapping::parse retains
        // that predicate as structured unresolved provenance, not satisfaction.
    }
    validate_generated(text, nup, toc, limits, token)
}
