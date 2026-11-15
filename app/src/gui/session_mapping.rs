//! Application-owned, reference-counted mapping files. Only our exact file and
//! empty private directory are removed; imported mappings are never owned here.
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug)]
pub(super) struct SessionMapping {
    root: PathBuf,
    pub path: PathBuf,
}
impl SessionMapping {
    pub fn create() -> std::io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for _ in 0..32 {
            let root = std::env::temp_dir().join(format!(
                "nxconvert-map-{}-{stamp:x}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&root) {
                Ok(()) => {
                    return Ok(Self {
                        path: root.join("mapping.nxnupmap"),
                        root,
                    })
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "Could not allocate session mapping storage",
        ))
    }
}
impl Drop for SessionMapping {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.root);
    }
}

#[derive(Default)]
pub(super) struct MappingBatch {
    pub pending: std::collections::VecDeque<PathBuf>,
    pub generated: usize,
    pub skipped: usize,
    pub errors: Vec<(PathBuf, String)>,
    pub active: Option<PathBuf>,
}
#[derive(Default)]
pub(super) struct BatchSummary {
    pub generated: usize,
    pub skipped: usize,
    pub cancelled: usize,
    pub errors: Vec<(PathBuf, String)>,
}
