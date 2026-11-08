//! Best-effort deallocation after a committed removal. A filesystem clone
//! isolates old readers. Windows can also exclude every other file handle.
use super::*;
use std::io;

pub(super) fn reclaim(
    path: &Path,
    source: File,
    lock: Option<same_file::Handle>,
    written: &RepackWrite,
    blocks: &[SourceBlock],
    old_slot: usize,
) -> u64 {
    // Use the new, validated index as the complete live set. Its metadata tail
    // and both header slots are outside the ranges sent to the filesystem.
    let result = platform::reclaim(path, source, lock, written, blocks, old_slot);
    result.unwrap_or(0)
}

pub(super) fn allocated_bytes(file: &File) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        file.metadata()
            .ok()
            .map(|metadata| metadata.blocks().saturating_mul(512))
    }
    #[cfg(windows)]
    {
        platform::allocated(file).ok()
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        None
    }
}

fn free_ranges(
    blocks: &[SourceBlock],
    tail: u64,
    alignment: u64,
) -> impl Iterator<Item = (u64, u64)> + '_ {
    let mut end = DATA_START;
    blocks
        .iter()
        .map(|block| (block.range.offset, block.range.offset + block.range.size))
        .chain(std::iter::once((tail, tail)))
        .filter_map(move |(start, next_end)| {
            let left = end.div_ceil(alignment) * alignment;
            let right = start / alignment * alignment;
            end = next_end;
            (left < right).then(|| (left, right - left))
        })
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
mod platform {
    use super::*;
    use std::os::{fd::AsRawFd, unix::fs::MetadataExt};

    pub(super) fn reclaim(
        path: &Path,
        source: File,
        _lock: Option<same_file::Handle>,
        written: &RepackWrite,
        blocks: &[SourceBlock],
        old_slot: usize,
    ) -> Result<u64, WriterError> {
        let metadata = source.metadata()?;
        // Atomic replacement would leave other hard links on the old inode.
        if metadata.nlink() != 1 {
            return Ok(0);
        }
        // SAFETY: stat is writable and source keeps the descriptor alive.
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstatvfs(source.as_raw_fd(), &mut stat) } != 0 || stat.f_frsize == 0 {
            return Ok(0);
        }
        let alignment = stat.f_frsize as u64;
        let mut ranges = free_ranges(blocks, written.tail_start, alignment).peekable();
        if ranges.peek().is_none() {
            return Ok(0);
        }
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let (mut staged, stage_path, _cleanup) = clone_file(parent, &source)?;
        commit_generation(&mut staged, &written.header, old_slot)?;
        for (offset, length) in ranges {
            punch(&staged, offset, length)?;
        }
        staged.sync_all()?;
        let released = metadata
            .blocks()
            .saturating_sub(staged.metadata()?.blocks())
            .saturating_mul(512);
        if released == 0 {
            return Ok(0);
        }
        drop(staged);
        atomic_replace(&stage_path, path)?;
        Ok(released)
    }

    #[cfg(target_os = "macos")]
    struct CloneGuard(PathBuf);
    #[cfg(target_os = "macos")]
    impl Drop for CloneGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(self.0.join("archive"));
            let _ = std::fs::remove_dir(&self.0);
        }
    }

    #[cfg(target_os = "macos")]
    fn clone_file(
        parent: &Path,
        source: &File,
    ) -> Result<(File, PathBuf, CloneGuard), WriterError> {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};

        let mut template = CString::new(parent.join(".nx-reclaim-XXXXXX").as_os_str().as_bytes())
            .map_err(|_| invalid("invalid reclaim staging path"))?
            .into_bytes_with_nul();
        // SAFETY: mkdtemp can modify the writable, NUL-terminated template.
        if unsafe { libc::mkdtemp(template.as_mut_ptr().cast()) }.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        let guard = CloneGuard(PathBuf::from(OsStr::from_bytes(
            &template[..template.len() - 1],
        )));
        let dir = File::open(&guard.0)?;
        // SAFETY: the descriptors are open and the name is NUL-terminated.
        if unsafe {
            libc::fclonefileat(
                source.as_raw_fd(),
                dir.as_raw_fd(),
                c"archive".as_ptr(),
                0x0004, // CLONE_ACL from <sys/clonefile.h>
            )
        } != 0
        {
            return Err(io::Error::last_os_error().into());
        }
        let path = guard.0.join("archive");
        Ok((
            OpenOptions::new().read(true).write(true).open(&path)?,
            path,
            guard,
        ))
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn clone_file(parent: &Path, source: &File) -> Result<(File, PathBuf, TempGuard), WriterError> {
        // FICLONE copies extents only. Preserve the original when we cannot
        // reproduce its ownership or when it has ACLs or other xattrs.
        let original = source.metadata()?;
        // SAFETY: a null buffer with length zero queries the attribute list size.
        if unsafe { libc::flistxattr(source.as_raw_fd(), std::ptr::null_mut(), 0) } != 0 {
            return Err(io::Error::from(io::ErrorKind::Unsupported).into());
        }
        let (file, guard) = unique_temp(parent, OsStr::new(".nx-reclaim"))?;
        // SAFETY: FICLONE receives the live source descriptor by value.
        if unsafe { libc::ioctl(file.as_raw_fd(), libc::FICLONE, source.as_raw_fd()) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        let cloned = file.metadata()?;
        if (original.uid(), original.gid()) != (cloned.uid(), cloned.gid()) {
            return Err(io::Error::from(io::ErrorKind::Unsupported).into());
        }
        file.set_permissions(original.permissions())?;
        Ok((file, guard.path.clone(), guard))
    }

    #[cfg(target_os = "macos")]
    fn punch(file: &File, offset: u64, length: u64) -> io::Result<()> {
        let mut range = libc::fpunchhole_t {
            fp_flags: 0,
            reserved: 0,
            fp_offset: offset as i64,
            fp_length: length as i64,
        };
        // SAFETY: the descriptor is live and range matches the fcntl ABI.
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_PUNCHHOLE, &mut range) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn punch(file: &File, offset: u64, length: u64) -> io::Result<()> {
        // SAFETY: the descriptor is live and ranges are bounded by INT64_MAX.
        if unsafe {
            libc::fallocate64(
                file.as_raw_fd(),
                libc::FALLOC_FL_PUNCH_HOLE | libc::FALLOC_FL_KEEP_SIZE,
                offset as i64,
                length as i64,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        ffi::c_void,
        os::windows::{
            fs::OpenOptionsExt,
            io::{AsRawHandle, RawHandle},
        },
    };

    const FSCTL_SET_SPARSE: u32 = 0x0009_00c4;
    const FSCTL_SET_ZERO_DATA: u32 = 0x0009_80c8;

    #[repr(C)]
    struct StandardInfo {
        allocation_size: i64,
        end_of_file: i64,
        links: u32,
        delete_pending: u8,
        directory: u8,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn DeviceIoControl(
            file: RawHandle,
            code: u32,
            input: *const c_void,
            input_size: u32,
            output: *mut c_void,
            output_size: u32,
            returned: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn GetFileInformationByHandleEx(
            file: RawHandle,
            class: u32,
            info: *mut c_void,
            size: u32,
        ) -> i32;
    }

    pub(super) fn allocated(file: &File) -> io::Result<u64> {
        // SAFETY: the buffer matches FILE_STANDARD_INFO and the handle is live.
        let mut info: StandardInfo = unsafe { std::mem::zeroed() };
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                1,
                (&mut info as *mut StandardInfo).cast(),
                std::mem::size_of::<StandardInfo>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(info.allocation_size as u64)
    }

    fn control(file: &File, code: u32, input: &[i64]) -> io::Result<()> {
        let mut returned = 0;
        let ptr = if input.is_empty() {
            std::ptr::null()
        } else {
            input.as_ptr().cast()
        };
        // SAFETY: the buffers and handle are live for this synchronous call.
        if unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                code,
                ptr,
                std::mem::size_of_val(input) as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn reclaim(
        path: &Path,
        source: File,
        lock: Option<same_file::Handle>,
        written: &RepackWrite,
        blocks: &[SourceBlock],
        old_slot: usize,
    ) -> Result<u64, WriterError> {
        // Zero whole 64 KiB units only. Windows can round smaller ranges
        // internally, so avoid issuing calls for small gaps.
        let mut ranges = free_ranges(blocks, written.tail_start, 65536).peekable();
        if ranges.peek().is_none() {
            return Ok(0);
        }
        // Close all existing editor handles before asking Windows to exclude
        // every other data handle. Revalidate both slots after that handoff.
        let original_headers = read_slots(&source)?;
        drop(source);
        drop(lock);
        let mut exclusive = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(path)?;
        if read_slots(&exclusive)? != original_headers
            || exclusive.metadata()?.len() != written.archive_size
        {
            return Ok(0);
        }
        let before = allocated(&exclusive)?;
        control(&exclusive, FSCTL_SET_SPARSE, &[])?;
        commit_generation(&mut exclusive, &written.header, old_slot)?;
        for (offset, length) in ranges {
            if control(
                &exclusive,
                FSCTL_SET_ZERO_DATA,
                &[offset as i64, (offset + length) as i64],
            )
            .is_err()
            {
                break;
            }
        }
        exclusive.sync_all()?;
        Ok(before.saturating_sub(allocated(&exclusive)?))
    }

    fn read_slots(file: &File) -> io::Result<[[u8; 256]; 2]> {
        use std::os::windows::fs::FileExt;
        let mut headers = [[0u8; 256]; 2];
        for (header, offset) in headers.iter_mut().zip(HEADER_SLOTS) {
            if file.seek_read(header, offset)? != header.len() {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
        }
        Ok(headers)
    }
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "android",
    windows
)))]
mod platform {
    use super::*;
    pub(super) fn reclaim(
        _: &Path,
        _: File,
        _: Option<same_file::Handle>,
        _: &RepackWrite,
        _: &[SourceBlock],
        _: usize,
    ) -> Result<u64, WriterError> {
        Ok(0)
    }
}
