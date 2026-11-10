//! Lightweight source discovery on the command worker, never on the UI thread.
use super::{Content, Source, SourceKind};
use nxformat_packer::{import, native, CancellationToken, Platform, SnapshotType};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

fn bounded(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let file = File::open(path).ok()?;
    if file.metadata().ok()?.len() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

fn display_text(nacp: &[u8]) -> (String, String) {
    if nacp.len() < 0x4000 {
        return Default::default();
    }
    let text = |field: &[u8]| {
        let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
        String::from_utf8_lossy(&field[..end]).trim().to_owned()
    };
    (0..16)
        .find_map(|i| {
            let start = i * 0x300;
            let name = text(&nacp[start..start + 0x200]);
            (!name.is_empty()).then(|| (name, text(&nacp[start + 0x200..start + 0x300])))
        })
        .unwrap_or_default()
}

fn extracted(path: &Path) -> Option<Source> {
    let candidates = import::discover(path, &import::ImportLimits::default()).ok()?;
    if candidates.is_empty() {
        return None;
    }
    let mut source = Source {
        path: path.to_owned(),
        kind: SourceKind::Extracted,
        supported: true,
        selected: true,
        ..Default::default()
    };
    for c in candidates {
        let root = if c.relative_root == "." {
            path.to_owned()
        } else {
            path.join(&c.relative_root)
        };
        let kind = if c.relative_root.starts_with("updates/") {
            "update"
        } else if c.relative_root.starts_with("dlc/") {
            "dlc"
        } else {
            "base"
        };
        let npdm = bounded(&root.join("exefs/main.npdm"), 1024 * 1024);
        let program = npdm.and_then(|bytes| {
            [
                import::structures::NpdmProfile::MetaAcid2048V1,
                import::structures::NpdmProfile::MetaAcid3072V1,
            ]
            .iter()
            .find_map(|&profile| import::structures::validate_npdm(&bytes, profile).ok())
            .map(|n| n.program_id)
        });
        let (name, publisher) = bounded(&root.join("control.nacp"), 1024 * 1024)
            .map(|b| display_text(&b))
            .unwrap_or_default();
        // Root artwork is optional for extracted layouts. No recursive media search.
        if source.icon.is_none() && kind != "dlc" {
            source.icon = native::NACP_LANGUAGE_MAP.iter().find_map(|(language, _)| {
                bounded(&root.join(format!("icon_{language}.dat")), 16 * 1024 * 1024)
            });
        }
        // updates/<version>/ and dlc/<id>/<version>/: the folder names the version.
        let version = if kind == "base" {
            0
        } else {
            root.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.parse().ok())
                .unwrap_or(0)
        };
        source.contents.push(Content {
            kind: kind.into(),
            application: program.unwrap_or(0),
            version,
            name,
            publisher,
            ..Default::default()
        });
    }
    // No base, one update: that update holds the whole game (standalone),
    // as in a dump whose base folder is empty. Its DLC may sit beside it.
    let has_base = source.contents.iter().any(|c| c.kind == "base");
    let mut updates = source.contents.iter_mut().filter(|c| c.kind == "update");
    if let (false, Some(update), None) = (has_base, updates.next(), updates.next()) {
        if update.application != 0 {
            update.kind = "standalone".into();
        }
    }
    if !source
        .contents
        .iter()
        .any(|c| (c.kind == "base" || c.kind == "standalone") && c.application != 0)
    {
        source.supported = false;
        source.selected = false;
        let base = path.join("base");
        source.issue = if base.is_dir() && !base.join("exefs/main.npdm").is_file() {
            "The base folder has no game in it (no exefs/main.npdm): this dump holds only updates or DLC. Add the base game to convert it."
        } else {
            "No base game found (no exefs/main.npdm). Add the base game, or use an import manifest for this layout."
        }
        .into();
    }
    Some(source)
}

fn is_nup(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("nup"))
}

fn nup(path: PathBuf, token: &CancellationToken) -> Source {
    let mut source = Source {
        kind: SourceKind::Nup,
        path,
        ..Default::default()
    };
    match import::inspect_nup(&source.path, &import::ImportLimits::for_nup(), token) {
        Ok(info) => {
            source.size = info.size;
            source.supported = true;
            source.selected = true;
            source.nup_platform = (info.platform != Platform::Unknown).then_some(info.platform);
            source.icon = info.icon;
            let title = info.title.unwrap_or_default();
            let publisher = info.publisher.unwrap_or_default();
            source.contents = info
                .packages
                .into_iter()
                .map(|package| {
                    let kind = match package.kind {
                        SnapshotType::Base => "base",
                        SnapshotType::Update => "update",
                        SnapshotType::StandaloneUpdate => "standalone",
                        SnapshotType::Dlc => "dlc",
                        SnapshotType::Unknown => "other",
                    };
                    Content {
                        kind: kind.into(),
                        application: info.application_id,
                        version: package.version,
                        display: package.display_version.unwrap_or_default(),
                        name: title.clone(),
                        publisher: publisher.clone(),
                        ..Default::default()
                    }
                })
                .collect();
        }
        Err(error) => source.issue = error.to_string(),
    }
    source
}

fn collect_nups(
    dir: &Path,
    depth: u32,
    options: &native::InventoryOptions,
    paths: &mut Vec<PathBuf>,
    token: &CancellationToken,
) -> Result<(), String> {
    if depth >= options.max_directory_depth {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        if token.is_cancelled() {
            return Err("Cancelled".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            collect_nups(&entry.path(), depth + 1, options, paths, token)?;
        } else if kind.is_file() && is_nup(&entry.path()) {
            if paths.len() >= options.max_sources {
                return Err("Too many NUP sources.".into());
            }
            paths.push(entry.path());
        }
    }
    Ok(())
}

pub fn scan(paths: Vec<PathBuf>, token: &CancellationToken) -> Result<Vec<Source>, String> {
    let mut result = Vec::new();
    let mut packages = Vec::new();
    let mut nups = Vec::new();
    let options = native::InventoryOptions::default();
    for path in paths {
        if token.is_cancelled() {
            return Err("Cancelled".into());
        }
        if path.is_dir() {
            if let Some(source) = extracted(&path) {
                result.push(source);
                continue;
            }
            collect_nups(&path, 0, &options, &mut nups, token)?;
        } else if is_nup(&path) {
            nups.push(path);
            continue;
        }
        packages.push(path);
    }
    nups.sort();
    nups.dedup();
    for path in nups {
        if token.is_cancelled() {
            return Err("Cancelled".into());
        }
        result.push(nup(path, token));
    }
    if !packages.is_empty() {
        let inventory = native::inventory(&packages, &options).map_err(|e| e.to_string())?;
        for s in inventory.sources {
            if token.is_cancelled() {
                return Err("Cancelled".into());
            }
            let supported = s.status == native::SourceStatus::Supported;
            let mut source = Source {
                path: s.path.clone(),
                size: s.size,
                firmware_bytes: s.system_update_bytes.unwrap_or_default(),
                supported,
                selected: supported,
                issue: s
                    .issues
                    .iter()
                    .filter(|i| i.severity == native::Severity::Error)
                    .map(|i| i.message.clone())
                    .collect::<Vec<_>>()
                    .join(" · "),
                ..Default::default()
            };
            for m in s.metas {
                let kind = match m.meta.meta_type {
                    native::ContentMetaType::Application => "base",
                    native::ContentMetaType::Patch => "update",
                    native::ContentMetaType::AddOnContent => "dlc",
                    _ => continue,
                };
                let mut content = Content {
                    kind: kind.into(),
                    application: m.meta.application_id,
                    version: m.meta.version,
                    platform: format!("{:?}", m.meta.generation),
                    ..Default::default()
                };
                if let Some(control) = m.meta.control {
                    let title = control
                        .titles
                        .iter()
                        .find(|t| t.name.as_ref().is_some_and(|s| !s.trim().is_empty()));
                    content.name = title.and_then(|t| t.name.clone()).unwrap_or_default();
                    content.publisher = title.and_then(|t| t.publisher.clone()).unwrap_or_default();
                    content.display = control.display_version.unwrap_or_default();
                    if source.icon.is_none() && kind != "dlc" {
                        let language = control
                            .titles
                            .iter()
                            .find(|t| t.name.as_ref().is_some_and(|s| !s.trim().is_empty()))
                            .map(|t| t.language.as_str());
                        let range = control
                            .icon_ranges
                            .iter()
                            .find(|r| Some(r.0.as_str()) == language)
                            .or_else(|| control.icon_ranges.first());
                        if let Some((_, offset, size, _)) = range {
                            if *size <= 16 * 1024 * 1024 {
                                if let Ok(mut file) = File::open(&s.path) {
                                    if file.seek(SeekFrom::Start(*offset)).is_ok() {
                                        let mut bytes = vec![0; *size as usize];
                                        if file.read_exact(&mut bytes).is_ok() {
                                            source.icon = Some(bytes);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                source.contents.push(content);
            }
            result.push(source);
        }
    }
    Ok(result)
}
