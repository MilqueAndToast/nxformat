use super::*;
use nxformat_packer::import::{
    self,
    synthetic_nup::{self as fixture, Node},
};
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

const APP: u64 = 0x0100000012340000;
const AUX: u64 = 0x0400000012340001;
struct Folder(PathBuf);
impl Folder {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nx-ui-flow-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn file(&self, name: &str, bytes: Vec<u8>) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        if let (Ok(path), Ok(parent)) = (self.0.canonicalize(), std::env::temp_dir().canonicalize())
        {
            if path.parent() == Some(parent.as_path())
                && path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(&format!("nx-ui-flow-{}-", std::process::id()))
            {
                let _ = fs::remove_dir_all(path);
            }
        }
    }
}

fn needs_mapping(version: u32, metadata: bool) -> Vec<u8> {
    let mut primary = fixture::base(APP);
    primary.version = version;
    let mut auxiliary = fixture::base(AUX);
    auxiliary.version = version;
    let mut cnmt = vec![0u8; 0x30 + 2 * 0x38];
    cnmt[..8].copy_from_slice(&APP.to_le_bytes());
    cnmt[8..12].copy_from_slice(&version.to_le_bytes());
    cnmt[0xc] = 0x80;
    cnmt[0xd] = 1;
    cnmt[0xe] = 0x10;
    cnmt[0x10] = 2;
    cnmt[0x20..0x28].copy_from_slice(&(APP + 0x800).to_le_bytes());
    for i in 0..2 {
        cnmt[0x30 + i * 0x38 + 0x36] = 1;
        cnmt[0x30 + i * 0x38 + 0x37] = i as u8;
    }
    fixture::build_with_extras(
        &[primary, auxiliary],
        true,
        &if metadata {
            vec![(
                0,
                vec![Node::Dir(
                    "meta",
                    vec![Node::File("Application.cnmt", cnmt)],
                )],
            )]
        } else {
            vec![]
        },
    )
}
fn unresolved() -> Vec<u8> {
    let mut nacp = fixture::nacp(&[(0, "Generated example")]);
    nacp[0x3070..0x3078].copy_from_slice(&(APP + 0x1000).to_le_bytes());
    nacp[0x3212] = 1;
    nacp[0x3216] = 1;
    nacp[0x3217] = 1;
    nacp[0x34d1] = 1;
    nacp[0x34d4] = 2;
    let mut program = fixture::base(AUX);
    program.meta = vec![(2, nacp.clone())];
    let mut meta = vec![0; 0x78];
    meta[..8].copy_from_slice(&(APP + 0x1801).to_le_bytes());
    meta[8..12].copy_from_slice(&7u32.to_le_bytes());
    meta[0xc] = 0x84;
    meta[0xd] = 1;
    meta[0xe] = 0x20;
    meta[0x10] = 1;
    meta[0x20..0x28].copy_from_slice(&(APP + 0x1001).to_le_bytes());
    meta[0x28..0x30].copy_from_slice(&APP.to_le_bytes());
    meta[0x30..0x34].copy_from_slice(&5u32.to_le_bytes());
    meta[0x76] = 2;
    fixture::build_with_extras(
        &[program, fixture::addon(APP)],
        true,
        &[
            (
                0,
                vec![Node::Dir(
                    "control",
                    vec![Node::Dir("Acd_1", vec![Node::File("control.nacp", nacp)])],
                )],
            ),
            (
                1,
                vec![Node::Dir("meta", vec![Node::File("DataPatch.cnmt", meta)])],
            ),
        ],
    )
}
fn source(path: &Path) -> Source {
    discovery::nup(path.to_owned(), None, &CancellationToken::default())
}
fn settle(app: &mut App) {
    super::tests::settle_outputs(app);
}
fn drive(app: &mut App, mut accept: impl FnMut(&MappingReview) -> bool) {
    let ctx = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        app.poll(&ctx);
        if !app.busy() {
            if let Some(review) = app.mapping_review.clone() {
                if accept(&review) {
                    app.accept_mapping_review();
                } else {
                    app.cancel_mapping_review();
                }
            } else if app.mapping_batch.is_none() {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "UI worker stalled: {}",
            app.status
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn generate(app: &mut App, path: &Path) {
    app.review_automatic_mapping(path.to_owned());
    drive(app, |_| true);
    assert!(
        app.sources
            .iter()
            .find(|s| s.path == path)
            .unwrap()
            .supported
    );
}

#[test]
fn mixed_batch_reviews_each_file_continues_errors_and_skips_valid_sources() {
    let dir = Folder::new();
    let bad = dir.file("bad.nup", needs_mapping(0, false));
    let plain = dir.file(
        "plain.nup",
        fixture::build(&[fixture::base(0x0400000055550000)], true),
    );
    let good = dir.file("good.nup", needs_mapping(0, true));
    let existing = dir.file("mapped.nup", needs_mapping(0, true));
    let unknown = dir.file("unknown.nup", unresolved());
    let mut app = App {
        sources: [&bad, &plain, &good, &existing, &unknown]
            .into_iter()
            .map(|p| source(p))
            .collect(),
        ..Default::default()
    };
    assert!(app.sources[1].supported);
    assert!(!app.sources[1].nup_mapping_needed);
    generate(&mut app, &existing);
    let previous = app
        .sources
        .iter()
        .find(|s| s.path == existing)
        .unwrap()
        .nup_mapping
        .clone()
        .unwrap();
    app.start_mapping_batch();
    let mut reviews = Vec::new();
    drive(&mut app, |review| {
        assert!(!review.preflight.violations.is_empty());
        if review.source == unknown {
            assert_eq!(review.preflight.unresolved_requirements.len(), 1);
        }
        reviews.push(review.source.clone());
        true
    });
    let summary = app.mapping_summary.as_ref().unwrap();
    assert_eq!(
        (summary.generated, summary.skipped, summary.errors.len()),
        (2, 2, 1)
    );
    assert_eq!(reviews, vec![bad.clone(), good.clone(), unknown.clone()]);
    assert_eq!(summary.errors[0].0, bad);
    let failed = app.sources.iter().find(|s| s.path == bad).unwrap();
    assert!(!failed.supported);
    assert!(failed
        .nup_mapping_feedback
        .as_ref()
        .unwrap()
        .contains("no embedded"));
    for p in [&good, &unknown, &existing] {
        let s = app.sources.iter().find(|s| &s.path == p).unwrap();
        assert!(s.supported);
        assert!(s.nup_generated_mapping.is_some());
        assert!(s.nup_mapping.as_ref().unwrap().is_file());
        assert_eq!(s.nup_acknowledged_toc, s.nup_toc);
        assert!(!p.with_extension("nxnupmap").exists());
    }
    assert_eq!(
        app.sources
            .iter()
            .find(|s| s.path == existing)
            .unwrap()
            .nup_mapping
            .as_ref(),
        Some(&previous)
    );
    settle(&mut app);
    app.convert();
    let unknown_job = app
        .queue
        .iter()
        .find(|j| j.args.contains(&text(&unknown)))
        .unwrap();
    assert!(unknown_job
        .args
        .iter()
        .any(|a| a == "--allow-unresolved-nup-requirements"));
}

#[test]
fn generated_mapping_is_owned_through_queue_and_cleaned_on_clear_retry_and_exit() {
    let dir = Folder::new();
    let path = dir.file("source.nup", needs_mapping(0, true));
    let bytes = fs::read(&path).unwrap();
    let mut app = App {
        sources: vec![source(&path)],
        ..Default::default()
    };
    generate(&mut app, &path);
    let map = app.sources[0].nup_mapping.clone().unwrap();
    let root = map.parent().unwrap().to_owned();
    settle(&mut app);
    app.convert();
    assert_eq!(app.queue.len(), 1);
    app.clear_sources();
    assert!(map.exists(), "queued job still needs mapping");
    app.queue.clear();
    assert!(!map.exists());
    assert!(!root.exists());
    app.sources.push(source(&path));
    generate(&mut app, &path);
    let second = app.sources[0].nup_mapping.clone().unwrap();
    assert_ne!(second, map);
    drop(app);
    assert!(!second.exists());
    assert!(!second.parent().unwrap().exists());
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn generated_mapping_cleans_after_conversion_and_failure_then_retry_works() {
    let dir = Folder::new();
    let path = dir.file("source.nup", needs_mapping(0, true));
    let original = fs::read(&path).unwrap();
    let mut app = App {
        sources: vec![source(&path)],
        ..Default::default()
    };
    generate(&mut app, &path);
    let badmap = app.sources[0].nup_mapping.clone().unwrap();
    fs::write(&badmap, "invalid mapping").unwrap();
    settle(&mut app);
    let output = app.games()[0].output.clone().unwrap();
    app.convert();
    drive(&mut app, |_| panic!("conversion must not invent consent"));
    assert!(!output.exists());
    assert!(!badmap.exists());
    assert!(!app.sources[0].supported);
    assert!(app.sources[0].nup_mapping.is_none());
    generate(&mut app, &path);
    let goodmap = app.sources[0].nup_mapping.clone().unwrap();
    settle(&mut app);
    app.convert();
    drive(&mut app, |_| panic!("already reviewed"));
    assert!(output.is_file());
    assert!(!goodmap.exists());
    assert!(app.sources.is_empty());
    nxformat_packer::inspect_archive(&output, &WriterOptions::default(), IntegrityPolicy::Strict)
        .unwrap();
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn unchanged_mapping_is_skipped_but_stale_source_needs_fresh_consent() {
    let dir = Folder::new();
    let path = dir.file("source.nup", needs_mapping(0, true));
    let mut app = App {
        sources: vec![source(&path)],
        ..Default::default()
    };
    generate(&mut app, &path);
    let oldmap = app.sources[0].nup_mapping.clone().unwrap();
    let oldtoc = app.sources[0].nup_acknowledged_toc.unwrap();
    app.start_mapping_batch();
    drive(&mut app, |_| panic!("unchanged mapping should be skipped"));
    assert_eq!(app.mapping_summary.as_ref().unwrap().skipped, 1);
    assert_eq!(app.sources[0].nup_mapping.as_ref(), Some(&oldmap));
    fs::write(&path, needs_mapping(1, true)).unwrap();
    app.start_mapping_batch();
    let mut reviewed = 0;
    drive(&mut app, |review| {
        assert_ne!(review.preflight.toc_sha256, oldtoc);
        reviewed += 1;
        true
    });
    assert_eq!(reviewed, 1);
    assert!(!oldmap.exists());
    assert_ne!(app.sources[0].nup_mapping.as_ref(), Some(&oldmap));
    assert_eq!(app.mapping_summary.as_ref().unwrap().generated, 1);
}

#[test]
fn batch_cancel_stops_unreviewed_files_and_remove_cleans_completed_artifacts() {
    let dir = Folder::new();
    let first = dir.file("first.nup", needs_mapping(0, true));
    let second = dir.file("second.nup", needs_mapping(0, true));
    let third = dir.file("third.nup", needs_mapping(0, true));
    let mut app = App {
        sources: [&first, &second, &third]
            .into_iter()
            .map(|p| source(p))
            .collect(),
        ..Default::default()
    };
    app.start_mapping_batch();
    drive(&mut app, |review| review.source == first);
    let summary = app.mapping_summary.as_ref().unwrap();
    assert_eq!((summary.generated, summary.cancelled), (1, 2));
    let map = app.sources[0].nup_mapping.clone().unwrap();
    assert!(map.exists());
    assert!(app.sources[1..].iter().all(|s| s.nup_mapping.is_none()));
    let game = app
        .games()
        .into_iter()
        .find(|g| g.key == text(&first))
        .unwrap();
    app.remove_game(&game);
    assert!(!map.exists());
    assert!(first.exists() && second.exists() && third.exists());
}

#[test]
fn ordinary_nup_without_cnmt_converts_without_mapping_or_review() {
    let dir = Folder::new();
    let path = dir.file(
        "plain.nup",
        fixture::build(&[fixture::base(0x0400000055550000)], true),
    );
    let mut app = App {
        sources: vec![source(&path)],
        ..Default::default()
    };
    assert!(app.sources[0].supported);
    assert!(!app.sources[0].nup_mapping_needed);
    app.start_mapping_batch();
    drive(&mut app, |_| {
        panic!("strict source needs no mapping consent")
    });
    let summary = app.mapping_summary.as_ref().unwrap();
    assert_eq!((summary.generated, summary.skipped), (0, 1));
    assert!(app.sources[0].nup_mapping_feedback.is_none());
    settle(&mut app);
    let output = app.games()[0].output.clone().unwrap();
    app.convert();
    assert!(!app.queue[0].args.iter().any(|a| a == "--nup-map"));
    drive(&mut app, |_| panic!("no mapping"));
    nxformat_packer::inspect_archive(output, &WriterOptions::default(), IntegrityPolicy::Strict)
        .unwrap();
}

#[test]
fn user_supplied_mapping_survives_batch_skip_conversion_and_clear() {
    let dir = Folder::new();
    let path = dir.file("source.nup", needs_mapping(0, true));
    let map = dir.0.join("user.nxnupmap");
    let data = import::generate_nup_mapping(
        &path,
        &import::ImportLimits::for_nup(),
        &CancellationToken::default(),
    )
    .unwrap();
    fs::write(&map, &data).unwrap();
    let mut s = discovery::nup(
        path.clone(),
        Some(map.clone()),
        &CancellationToken::default(),
    );
    s.nup_acknowledged_toc = s.nup_toc;
    let mut app = App {
        sources: vec![s],
        ..Default::default()
    };
    app.start_mapping_batch();
    drive(&mut app, |_| panic!("valid unchanged imported mapping"));
    assert_eq!(app.mapping_summary.as_ref().unwrap().skipped, 1);
    assert!(app.sources[0].nup_generated_mapping.is_none());
    settle(&mut app);
    app.convert();
    drive(&mut app, |_| panic!("review retained"));
    app.clear_sources();
    assert_eq!(fs::read_to_string(&map).unwrap(), data);
    assert!(path.exists());
}

#[test]
fn output_checks_follow_folder_title_existing_file_and_queue_only_skip() {
    let dir = Folder::new();
    let path = dir.file(
        "plain.nup",
        fixture::build(&[fixture::base(0x0400000055550000)], true),
    );
    let mut app = App {
        sources: vec![source(&path)],
        ..Default::default()
    };
    settle(&mut app);
    let initial = app.games()[0].planned_output.clone().unwrap();
    assert_eq!(app.games()[0].output_state, outputs::OutputState::Missing);
    fs::write(&initial, b"not an NX; existence only").unwrap();
    settle(&mut app);
    assert_eq!(app.games()[0].output_state, outputs::OutputState::Exists);
    assert!(app.games()[0].output.is_none());
    app.convert();
    assert!(app.queue.is_empty());
    let key = app.games()[0].key.clone();
    app.appearances.insert(
        key,
        Appearance {
            title: "Different output title".into(),
            ..Default::default()
        },
    );
    settle(&mut app);
    assert_eq!(app.games()[0].output_state, outputs::OutputState::Missing);
    assert_ne!(app.games()[0].planned_output.as_ref(), Some(&initial));
    let folder = dir.0.join("new-output");
    fs::create_dir(&folder).unwrap();
    app.output_dir = Some(folder);
    settle(&mut app);
    let target = app.games()[0].planned_output.clone().unwrap();
    fs::write(&target, b"existing output").unwrap();
    settle(&mut app);
    app.skip_existing();
    assert!(app.sources.is_empty());
    assert_eq!(fs::read(target).unwrap(), b"existing output");
    assert_eq!(fs::read(initial).unwrap(), b"not an NX; existence only");
    assert!(path.exists());
}

#[test]
fn occupied_output_paths_and_collisions_block_without_deleting_or_skipping() {
    let dir = Folder::new();
    let one = dir.file(
        "one.nup",
        fixture::build(&[fixture::base(0x0400000055550000)], true),
    );
    let two = dir.file(
        "two.nup",
        fixture::build(&[fixture::base(0x0400000055550000)], true),
    );
    let mut app = App {
        sources: vec![source(&one), source(&two)],
        ..Default::default()
    };
    settle(&mut app);
    assert!(app
        .games()
        .iter()
        .all(|g| g.output.is_none() && g.problem.contains("same output path")));
    app.convert();
    assert!(app.queue.is_empty());
    app.skip_existing();
    assert_eq!(app.sources.len(), 2);
    app.sources.pop();
    settle(&mut app);
    let output = app.games()[0].planned_output.clone().unwrap();
    fs::create_dir(&output).unwrap();
    settle(&mut app);
    assert!(matches!(
        app.games()[0].output_state,
        outputs::OutputState::Blocked(_)
    ));
    app.skip_existing();
    assert_eq!(app.sources.len(), 1);
    assert!(output.is_dir());
    app.output_dir = Some(dir.file("occupied-parent", b"keep".to_vec()));
    settle(&mut app);
    assert!(matches!(
        app.games()[0].output_state,
        outputs::OutputState::Blocked(_)
    ));
    app.convert();
    assert!(app.queue.is_empty());
    assert_eq!(fs::read(dir.0.join("occupied-parent")).unwrap(), b"keep");
}

#[test]
fn closing_app_cancels_and_joins_reader_before_last_owned_mapping_cleanup() {
    let owned = Arc::new(session_mapping::SessionMapping::create().unwrap());
    let path = owned.path.clone();
    fs::write(&path, "owned fixture").unwrap();
    let token = CancellationToken::default();
    let worker_token = token.clone();
    let resource = owned.clone();
    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done = finished.clone();
    let handle = std::thread::spawn(move || {
        while !worker_token.is_cancelled() {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(resource.path.exists());
        drop(resource);
        done.store(true, Ordering::SeqCst);
    });
    let (_tx, rx) = mpsc::channel();
    let app = App {
        sources: vec![Source {
            nup_generated_mapping: Some(owned.clone()),
            ..Default::default()
        }],
        running: Some(Running {
            job: Job {
                label: "Fixture".into(),
                args: vec![],
                purpose: Purpose::Pack,
                display: None,
                metadata: None,
                source_bytes: None,
                mapping_resources: vec![owned.clone()],
                nup_reviewed_toc: None,
            },
            rx,
            cancel: token.clone(),
            total: 0,
            done: 0,
            lines: vec![],
            started: Instant::now(),
            mapping_skipped: false,
            worker: Some(WorkerGuard {
                cancel: token,
                handle: Some(handle),
            }),
        }),
        ..Default::default()
    };
    drop(owned);
    drop(app);
    assert!(finished.load(Ordering::SeqCst));
    assert!(!path.exists());
    assert!(!path.parent().unwrap().exists());
}

#[test]
fn queued_conversion_rechecks_reviewed_source_even_if_mapping_was_replaced() {
    let dir = Folder::new();
    let path = dir.file("source.nup", needs_mapping(0, true));
    let mut app = App {
        sources: vec![source(&path)],
        ..Default::default()
    };
    generate(&mut app, &path);
    let map = app.sources[0].nup_mapping.clone().unwrap();
    let reviewed = app.sources[0].nup_acknowledged_toc;
    settle(&mut app);
    let output = app.games()[0].output.clone().unwrap();
    app.convert();
    assert_eq!(app.queue[0].nup_reviewed_toc, reviewed);
    fs::write(&path, needs_mapping(1, true)).unwrap();
    let replacement = import::generate_nup_mapping(
        &path,
        &import::ImportLimits::for_nup(),
        &CancellationToken::default(),
    )
    .unwrap();
    fs::write(&map, replacement).unwrap();
    drive(&mut app, |_| {
        panic!("conversion cannot silently review changed input")
    });
    assert!(!output.exists());
    assert!(!map.exists());
    assert!(app
        .log
        .iter()
        .any(|l| l.contains("changed since its warnings were reviewed")));
    assert!(!app.sources[0].supported);
}

#[test]
fn source_toolbar_and_existing_badge_fit_and_ordinary_nup_has_no_mapping_controls() {
    fn texts(shape: &egui::epaint::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::epaint::Shape::Vec(shapes) => {
                for s in shapes {
                    texts(s, out)
                }
            }
            egui::epaint::Shape::Text(s) => out.push((
                s.galley.job.text.clone(),
                egui::Rect::from_min_size(s.pos, s.galley.size()),
            )),
            _ => {}
        }
    }
    let dir = Folder::new();
    let path = dir.file(
        "plain.nup",
        fixture::build(&[fixture::base(0x0400000055550000)], true),
    );
    let mut app = App {
        sources: vec![source(&path)],
        ..Default::default()
    };
    settle(&mut app);
    let output = app.games()[0].planned_output.clone().unwrap();
    fs::write(&output, b"existence only").unwrap();
    settle(&mut app);
    for width in [440.0, 636.0, 900.0] {
        let ctx = egui::Context::default();
        theme::setup(&ctx);
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 700.0));
        let mut labels = vec![];
        for _ in 0..3 {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(viewport),
                    ..Default::default()
                },
                |ui| app.source_list(ui),
            );
            labels.clear();
            for s in &output.shapes {
                texts(&s.shape, &mut labels);
            }
            output.drop_without_applying_deltas();
        }
        let rect = |label: &str| {
            labels
                .iter()
                .find(|(text, _)| text == label)
                .unwrap_or_else(|| {
                    panic!(
                        "Missing {label}: {:?}",
                        labels.iter().map(|(t, _)| t).collect::<Vec<_>>()
                    )
                })
                .1
        };
        for label in [
            "Files",
            "Folder",
            "Skip Already Existing",
            "Clear",
            "Already exists",
            "Configure",
        ] {
            assert!(
                viewport.contains_rect(rect(label)),
                "{label} clipped at width {width}: {:?}",
                rect(label)
            );
        }
        assert!(rect("Files").center().x < rect("Folder").center().x);
        assert!(rect("Folder").center().x < rect("Skip Already Existing").center().x);
        assert!(rect("Skip Already Existing").center().x < rect("Clear").center().x);
        assert!(rect("Already exists").center().x < rect("Configure").center().x);
        assert!(!labels
            .iter()
            .any(|(t, _)| t.contains("Generate NUP") || t.contains("Load mapping")));
    }
}

#[test]
#[ignore = "Creates synthetic files for a native UI check in a new explicitly selected folder"]
fn export_visual_fixture_set() {
    let root = PathBuf::from(
        std::env::var_os("NXCONVERT_UI_FIXTURE_DIR")
            .expect("set NXCONVERT_UI_FIXTURE_DIR to a new private folder"),
    );
    fs::create_dir(&root).unwrap();
    for (name, bytes) in [
        ("01-complete.nup", needs_mapping(0, true)),
        ("02-missing-metadata.nup", needs_mapping(0, false)),
        ("03-unresolved.nup", unresolved()),
        (
            "04-direct.nup",
            fixture::build(&[fixture::base(0x0400000055550000)], true),
        ),
    ] {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(name))
            .unwrap();
        std::io::Write::write_all(&mut f, &bytes).unwrap();
    }
    let app = App {
        sources: vec![source(&root.join("04-direct.nup"))],
        ..Default::default()
    };
    let planned = app.games()[0].planned_output.clone().unwrap();
    fs::write(
        planned,
        b"Synthetic existing output; deliberately not verified as NX",
    )
    .unwrap();
}
