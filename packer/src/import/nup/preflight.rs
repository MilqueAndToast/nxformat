use super::*;

/// Detected representational NUP-v1 violations; not a complete conformance
/// certificate. Corrupt structure or unreadable metadata remains a hard error.
#[derive(Clone, Debug)]
pub struct NupPreflight {
    pub toc_sha256: [u8; 32],
    pub violations: Vec<String>,
}

pub fn inspect_nup_spec(
    path: &Path,
    limits: &ImportLimits,
    token: &crate::CancellationToken,
) -> Result<NupPreflight, WriterError> {
    limits.validate()?;
    let (toc, _) = read_toc(&mut File::open(path)?, limits)?;
    if toc.packages.len() > limits.max_snapshots {
        return Err(limit("NUP snapshot budget"));
    }
    let mut violations = Vec::new();
    let bases = toc.packages.iter().filter(|p| p.kind == 0).count();
    if bases > 1 {
        violations.push(format!(
            "The package table contains {bases} BASE records; NUP v1 §3.1.2 permits at most one."
        ));
    }
    let mut entries = 0;
    let mut metadata_bytes = 0u64;
    for (i, p) in toc.packages.iter().enumerate() {
        cancelled(token)?;
        if p.kind > 2 || p.flags > 1 || (p.kind != 1 && p.flags != 0) {
            return Err(bad(&format!("package-{i}: unsupported/invalid NUP kind or flags; compatibility cannot override this")));
        }
        let mut raw = Tree::default();
        toc.walk(p.root, "", &mut raw, &mut BTreeSet::new(), 1, limits)?;
        charge_entries(&mut entries, &raw, MAX_RESOLVED_ENTRIES)?;
        let roots: BTreeSet<_> = raw
            .files
            .keys()
            .chain(raw.dirs.iter())
            .filter_map(|p| p.split('/').next())
            .filter(|p| !["exefs", "romfs", "data"].contains(p))
            .collect();
        if !roots.is_empty() {
            violations.push(format!("package-{i} has content outside exefs/, romfs/ or data/ ({}); NUP v1 §3.1.3 defines those canonical roots.",roots.into_iter().take(8).collect::<Vec<_>>().join(", ")));
        }
        if p.kind == 1 && ((bases == 0 && p.flags != 1) || (bases > 0 && p.flags != 0)) {
            violations.push(format!("package-{i}: standalone-update flag disagrees with the presence of a BASE (§3.1.2)."));
        }
        if raw
            .files
            .keys()
            .any(|p| p.split('/').any(|n| n == ".listing"))
        {
            violations.push(format!(
                "package-{i} contains a reserved .listing dump artifact (§6.3)."
            ));
        }
        for (name, index) in &raw.files {
            if name.starts_with("meta/") && name.ends_with(".cnmt") {
                metadata_bytes = metadata_bytes
                    .checked_add(toc.files[*index].size)
                    .filter(|n| *n <= limits.max_inventory_bytes)
                    .ok_or_else(|| limit("CNMT preflight budget"))?;
                let bytes =
                    compatibility::small_file(path, &toc.files[*index], token, 1024 * 1024)?;
                let meta = crate::native::nca::parse_cnmt(&bytes)
                    .map_err(|e| bad(&format!("package-{i}: invalid embedded CNMT: {e}")))?;
                if matches!(
                    meta.meta_type,
                    crate::native::ContentMetaType::Application
                        | crate::native::ContentMetaType::Patch
                        | crate::native::ContentMetaType::DataPatch
                ) && meta.version != p.version
                {
                    violations.push(format!("package-{i} declares version {} but its native CNMT records version {}; NUP v1 §3.1.1 requires the original numeric content version.",p.version,meta.version));
                }
            }
        }
    }
    Ok(NupPreflight {
        toc_sha256: toc.digest,
        violations,
    })
}
