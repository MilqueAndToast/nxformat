//! Sources read by the caller instead of opened from disk.
//!
//! An emulator that can decrypt games registers a source with a size and a
//! positioned read; nxformat gets a path for it (`nxsource://<id>/<name>`)
//! that every command accepts where it accepts a file. nxformat never sees
//! keys: it only reads the plaintext bytes the caller returns.
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const SCHEME: &str = "nxsource://";

/// Positioned reads of one plaintext source. Calls for one source are
/// serialized, so an implementation needs no locking of its own.
pub trait SourceReader: Send {
    /// Fill `buffer` from `offset`; a short or failed read is an error.
    fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> io::Result<()>;
}

pub(crate) struct Registered {
    size: u64,
    reader: Mutex<Box<dyn SourceReader>>,
}

fn registry() -> &'static Mutex<HashMap<PathBuf, Arc<Registered>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Arc<Registered>>>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

/// A registered source; it is removed when this value is dropped. Commands
/// that are still reading keep their own reference until they finish.
pub struct SourceRegistration {
    path: PathBuf,
}

impl SourceRegistration {
    /// The path to pass to commands. `name` is the file name shown in
    /// messages and used to recognise the package type (`.nsp`, `.xci`).
    pub fn new(name: &str, size: u64, reader: Box<dyn SourceReader>) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let name: String = name
            .chars()
            .map(|c| {
                if matches!(c, '/' | '\\' | ':') {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        let path = PathBuf::from(format!("{SCHEME}{id}/{name}"));
        registry().lock().unwrap().insert(
            path.clone(),
            Arc::new(Registered {
                size,
                reader: Mutex::new(reader),
            }),
        );
        Self { path }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SourceRegistration {
    fn drop(&mut self) {
        registry().lock().unwrap().remove(&self.path);
    }
}

fn lookup(path: &Path) -> Option<Arc<Registered>> {
    if !path.to_str().is_some_and(|p| p.starts_with(SCHEME)) {
        return None;
    }
    registry().lock().unwrap().get(path).cloned()
}

/// True for a registered caller source.
pub(crate) fn is_virtual(path: &Path) -> bool {
    lookup(path).is_some()
}

/// A source file on disk or read by the caller.
pub(crate) enum SourceFile {
    Disk(File),
    Virtual {
        source: Arc<Registered>,
        position: u64,
    },
}

impl SourceFile {
    pub(crate) fn len(&self) -> io::Result<u64> {
        match self {
            Self::Disk(file) => Ok(file.metadata()?.len()),
            Self::Virtual { source, .. } => Ok(source.size),
        }
    }
}

/// Open a source: a registered caller source, else a file on disk.
pub(crate) fn open(path: &Path) -> io::Result<SourceFile> {
    match lookup(path) {
        Some(source) => Ok(SourceFile::Virtual {
            source,
            position: 0,
        }),
        None => File::open(path).map(SourceFile::Disk),
    }
}

impl Read for SourceFile {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Disk(file) => file.read(buffer),
            Self::Virtual { source, position } => {
                let left = source.size.saturating_sub(*position);
                let n = (buffer.len() as u64).min(left) as usize;
                if n == 0 {
                    return Ok(0);
                }
                source
                    .reader
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .read_at(*position, &mut buffer[..n])?;
                *position += n as u64;
                Ok(n)
            }
        }
    }
}

impl Seek for SourceFile {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        match self {
            Self::Disk(file) => file.seek(to),
            Self::Virtual { source, position } => {
                let target = match to {
                    SeekFrom::Start(v) => Some(v),
                    SeekFrom::End(d) => source.size.checked_add_signed(d),
                    SeekFrom::Current(d) => position.checked_add_signed(d),
                };
                *position = target.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "seek before the start")
                })?;
                Ok(*position)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Bytes(Vec<u8>);
    impl SourceReader for Bytes {
        fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
            let at = offset as usize;
            buffer.copy_from_slice(&self.0[at..at + buffer.len()]);
            Ok(())
        }
    }

    #[test]
    fn reads_seeks_and_unregisters() {
        let data: Vec<u8> = (0..=255).collect();
        let reg = SourceRegistration::new("game.nsp", 256, Box::new(Bytes(data)));
        assert_eq!(reg.path().extension().unwrap(), "nsp");
        let mut file = open(reg.path()).unwrap();
        assert_eq!(file.len().unwrap(), 256);
        file.seek(SeekFrom::Start(250)).unwrap();
        let mut rest = Vec::new();
        file.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, [250, 251, 252, 253, 254, 255]);
        let path = reg.path().to_owned();
        drop(reg);
        assert!(!is_virtual(&path));
    }
}
