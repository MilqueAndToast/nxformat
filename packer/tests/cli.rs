//! nxconvert commands run in process, as a frontend runs them.
use nxformat_packer::{cli, CancellationToken};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures")
        .join(name)
}

type Lines = Arc<Mutex<Vec<(bool, String)>>>;

fn collect() -> (cli::Sink, Lines) {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let store = lines.clone();
    let sink: cli::Sink =
        Arc::new(move |error, line: &str| store.lock().unwrap().push((error, line.to_owned())));
    (sink, lines)
}

#[test]
fn contents_lines_go_to_the_sink() {
    let (sink, lines) = collect();
    let archive = fixture("dlc_only_application.nx");
    cli::run(
        vec!["contents".into(), archive.display().to_string()],
        Some(sink),
        None,
    )
    .unwrap();
    let lines = lines.lock().unwrap();
    let size = std::fs::metadata(&archive).unwrap().len();
    assert!(lines.iter().any(|(e, l)| !e && l.starts_with("@APP\t")));
    assert!(lines.contains(&(false, format!("@SIZE\t{size}"))));
}

#[test]
fn a_cancelled_edit_leaves_the_archive_as_it_was() {
    let dir = std::env::temp_dir().join(format!("nxformat-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let archive = dir.join("shared_with_orphan_block.nx");
    std::fs::copy(fixture("shared_with_orphan_block.nx"), &archive).unwrap();
    let before = std::fs::read(&archive).unwrap();
    let token = CancellationToken::default();
    token.cancel();
    let (sink, _) = collect();
    let result = cli::run(
        vec![
            "compact".into(),
            archive.display().to_string(),
            "--frontend".into(),
        ],
        Some(sink),
        Some(token),
    );
    assert!(result.is_err());
    assert_eq!(std::fs::read(&archive).unwrap(), before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_sink_is_released_after_a_run() {
    let (sink, lines) = collect();
    let archive = fixture("dlc_only_application.nx").display().to_string();
    cli::run(vec!["contents".into(), archive.clone()], Some(sink), None).unwrap();
    let count = lines.lock().unwrap().len();
    // A second run without a sink prints to stdout, not to the old sink.
    cli::run(vec!["contents".into(), archive], None, None).unwrap();
    assert_eq!(lines.lock().unwrap().len(), count);
}

#[test]
fn nup_cli_checks_disk_limits_before_decoding() {
    use nxformat_packer::import::synthetic_nup::{base, build, Node};
    let dir = std::env::temp_dir().join(format!("nxformat-cli-nup-budget-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let nup = dir.join("game.nup");
    let out = dir.join("game.nx");
    let mut game = base(0x0400_0000_0000_1000);
    // RomFS is streamed from the NUP; only ExeFS is staged on disk.
    game.exefs
        .get_or_insert_with(Vec::new)
        .push(Node::File("large.bin", vec![0; 2 * 1024 * 1024]));
    std::fs::write(&nup, build(&[game], true)).unwrap();
    let (sink, _) = collect();
    let result = cli::run(
        vec![
            "pack-nup".into(),
            nup.display().to_string(),
            out.display().to_string(),
            "--platform".into(),
            "switch2".into(),
            "--temp-mib".into(),
            "1".into(),
        ],
        Some(sink),
        None,
    );
    assert!(result.unwrap_err().to_string().contains("staging budget"));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn nup_cli_honors_the_callers_cancellation_before_staging() {
    use nxformat_packer::import::synthetic_nup::{base, build};
    let dir = std::env::temp_dir().join(format!("nxformat-cli-nup-cancel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let nup = dir.join("game.nup");
    std::fs::write(&nup, build(&[base(0x0400_0000_0000_1000)], true)).unwrap();
    let token = CancellationToken::default();
    token.cancel();
    let (sink, _) = collect();
    let result = cli::run(
        vec![
            "pack-nup".into(),
            nup.display().to_string(),
            dir.join("game.nx").display().to_string(),
            "--platform".into(),
            "switch2".into(),
        ],
        Some(sink),
        Some(token),
    );
    assert!(result.unwrap_err().to_string().contains("cancelled"));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}
