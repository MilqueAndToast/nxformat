//! The C entry points, called as a C frontend calls them.
use nxformat::*;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::PathBuf;

fn fixture(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures")
        .join(name)
        .display()
        .to_string()
}

unsafe extern "C" fn collect(user: *mut c_void, is_error: c_int, line: *const c_char) {
    let lines = &mut *user.cast::<Vec<(bool, String)>>();
    lines.push((
        is_error != 0,
        CStr::from_ptr(line).to_str().unwrap().to_owned(),
    ));
}

fn run(args: &[String], cancel: *mut nxformat_cancel) -> (c_int, Vec<(bool, String)>, String) {
    let owned: Vec<CString> = args
        .iter()
        .map(|a| CString::new(a.as_str()).unwrap())
        .collect();
    let ptrs: Vec<*const c_char> = owned.iter().map(|a| a.as_ptr()).collect();
    let mut lines: Vec<(bool, String)> = Vec::new();
    let mut error = [0 as c_char; 256];
    let code = unsafe {
        nxformat_run(
            ptrs.as_ptr(),
            ptrs.len(),
            Some(collect),
            (&mut lines as *mut Vec<(bool, String)>).cast(),
            cancel,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    let error = unsafe { CStr::from_ptr(error.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    (code, lines, error)
}

#[test]
fn reports_its_version() {
    assert_eq!(nxformat_abi_version(), 1);
    let version = unsafe { CStr::from_ptr(nxformat_version()) };
    assert_eq!(version.to_str().unwrap(), env!("CARGO_PKG_VERSION"));
}

#[test]
fn contents_lines_reach_the_callback() {
    let (code, lines, _) = run(
        &["contents".into(), fixture("dlc_only_application.nx")],
        std::ptr::null_mut(),
    );
    assert_eq!(code, NXFORMAT_OK);
    assert!(lines.iter().any(|(e, l)| !e && l.starts_with("@APP\t")));
}

#[test]
fn a_failure_fills_the_error_text() {
    let (code, _, error) = run(
        &["contents".into(), fixture("no_such_file.nx")],
        std::ptr::null_mut(),
    );
    assert_eq!(code, NXFORMAT_FAILED);
    assert!(!error.is_empty());
}

#[test]
fn a_cancelled_run_reports_cancelled() {
    let dir = std::env::temp_dir().join(format!("nxformat-ffi-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let archive = dir.join("shared_with_orphan_block.nx");
    std::fs::copy(fixture("shared_with_orphan_block.nx"), &archive).unwrap();
    let before = std::fs::read(&archive).unwrap();
    let cancel = nxformat_cancel_new();
    unsafe { nxformat_cancel_set(cancel) };
    let (code, _, _) = run(
        &[
            "compact".into(),
            archive.display().to_string(),
            "--frontend".into(),
        ],
        cancel,
    );
    unsafe { nxformat_cancel_free(cancel) };
    assert_eq!(code, NXFORMAT_CANCELLED);
    assert_eq!(std::fs::read(&archive).unwrap(), before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn null_and_bad_arguments_are_refused() {
    let code = unsafe {
        nxformat_run(
            std::ptr::null(),
            usize::MAX,
            None,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    assert_eq!(code, NXFORMAT_INVALID);
    let args = [std::ptr::null::<c_char>()];
    let code = unsafe {
        nxformat_run(
            args.as_ptr(),
            usize::MAX,
            None,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    assert_eq!(code, NXFORMAT_INVALID);
    let code = unsafe {
        nxformat_run(
            args.as_ptr(),
            1,
            None,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    assert_eq!(code, NXFORMAT_INVALID);
    let bad = [0xffu8, 0];
    let args = [bad.as_ptr().cast::<c_char>()];
    let code = unsafe {
        nxformat_run(
            args.as_ptr(),
            1,
            None,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    assert_eq!(code, NXFORMAT_INVALID);
}

unsafe extern "C" fn read_bytes(
    user: *mut c_void,
    offset: u64,
    buffer: *mut c_void,
    size: usize,
) -> c_int {
    let bytes = &*user.cast::<Vec<u8>>();
    let Some(range) = bytes.get(offset as usize..offset as usize + size) else {
        return 1;
    };
    std::ptr::copy_nonoverlapping(range.as_ptr(), buffer.cast(), size);
    0
}

fn source(name: &str, bytes: &mut Vec<u8>) -> *mut nxformat_source {
    let name = CString::new(name).unwrap();
    unsafe {
        nxformat_source_new(
            name.as_ptr(),
            bytes.len() as u64,
            Some(read_bytes),
            (bytes as *mut Vec<u8>).cast(),
        )
    }
}

fn path(source: *const nxformat_source) -> String {
    unsafe { CStr::from_ptr(nxformat_source_path(source)) }
        .to_str()
        .unwrap()
        .to_owned()
}

#[test]
fn packs_and_adds_from_sources_the_caller_reads() {
    use nxformat_packer::native::synthetic::{dnsp, title_entries, TitleSpec};
    const APP: u64 = 0x0100_0000_0001_0000;
    let mut base = dnsp(&title_entries(&TitleSpec::application(APP, 0, 1)));
    let mut update = dnsp(&title_entries(&TitleSpec::patch(APP, 65536, 2)));
    let base_source = source("game.nsp", &mut base);
    let update_source = source("update.nsp", &mut update);
    assert!(!base_source.is_null() && !update_source.is_null());
    let dir = std::env::temp_dir().join(format!("nxformat-ffi-sources-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let nx = dir.join("game.nx").display().to_string();

    let (code, _, error) = run(
        &["pack".into(), path(base_source), nx.clone()],
        std::ptr::null_mut(),
    );
    assert_eq!(code, NXFORMAT_OK, "{error}");
    let (code, lines, error) = run(
        &["add".into(), nx.clone(), path(update_source)],
        std::ptr::null_mut(),
    );
    assert_eq!(code, NXFORMAT_OK, "{error}");
    assert!(lines.iter().any(|(_, l)| l.starts_with("@ADDED\tupdate")));
    let (_, lines, _) = run(&["contents".into(), nx.clone()], std::ptr::null_mut());
    assert_eq!(
        lines
            .iter()
            .filter(|(_, l)| l.starts_with("@SNAPSHOT"))
            .count(),
        2
    );

    // A source whose reads fail makes the command fail, not crash.
    let mut short = base[..base.len() / 2].to_vec();
    let broken = unsafe {
        let name = CString::new("broken.nsp").unwrap();
        nxformat_source_new(
            name.as_ptr(),
            base.len() as u64,
            Some(read_bytes),
            (&mut short as *mut Vec<u8>).cast(),
        )
    };
    let other = dir.join("broken.nx").display().to_string();
    let (code, _, _) = run(&["pack".into(), path(broken), other], std::ptr::null_mut());
    assert_ne!(code, NXFORMAT_OK);

    let stale = path(base_source);
    unsafe {
        nxformat_source_free(base_source);
        nxformat_source_free(update_source);
        nxformat_source_free(broken);
    }
    let (code, _, _) = run(
        &[
            "pack".into(),
            stale,
            dir.join("gone.nx").display().to_string(),
        ],
        std::ptr::null_mut(),
    );
    assert_ne!(code, NXFORMAT_OK);
    assert!(unsafe {
        nxformat_source_new(std::ptr::null(), 0, Some(read_bytes), std::ptr::null_mut())
    }
    .is_null());
    let _ = std::fs::remove_dir_all(&dir);
}
