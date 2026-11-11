//! Bounded, seekable composition of physical ranges and logical zero-filled gaps.
use super::{bad, range, SourceReader};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
};

pub(super) enum Content {
    Disk(u64),
    Zero,
    Reader(Box<dyn SourceReader>),
}
struct Part {
    start: u64,
    end: u64,
    content: Content,
}
pub(super) struct View {
    file: File,
    physical_size: u64,
    parts: Vec<Part>,
    pub size: u64,
}
impl View {
    pub fn new(file: &File) -> io::Result<Self> {
        Ok(Self {
            file: file.try_clone()?,
            physical_size: file.metadata()?.len(),
            parts: Vec::new(),
            size: 0,
        })
    }
    pub fn append(&mut self, size: u64, content: Content) -> io::Result<()> {
        if size == 0 {
            return Ok(());
        }
        let end = range(self.size, size, i64::MAX as u64)?;
        if let Content::Disk(at) = content {
            range(at, size, self.physical_size)?;
        }
        self.parts.push(Part {
            start: self.size,
            end,
            content,
        });
        self.size = end;
        Ok(())
    }
}
impl SourceReader for View {
    fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
        let end = range(offset, buffer.len() as u64, self.size)?;
        let first = self.parts.partition_point(|p| p.end <= offset);
        for part in self.parts[first..].iter_mut().take_while(|p| p.start < end) {
            let a = offset.max(part.start);
            let b = end.min(part.end);
            let out = &mut buffer[(a - offset) as usize..(b - offset) as usize];
            match &mut part.content {
                Content::Disk(at) => {
                    self.file.seek(SeekFrom::Start(*at + a - part.start))?;
                    self.file.read_exact(out)?;
                }
                Content::Zero => out.fill(0),
                Content::Reader(reader) => reader.read_at(a - part.start, out)?,
            }
        }
        Ok(())
    }
}

/// Replaces disjoint physical ranges while retaining every other byte. Container
/// entry offsets/sizes are patched separately; originals are always read-only.
pub(super) fn splice(
    file: &File,
    start: u64,
    size: u64,
    mut replacements: Vec<(u64, u64, u64, Box<dyn SourceReader>)>,
) -> io::Result<View> {
    let end = range(start, size, file.metadata()?.len())?;
    replacements.sort_by_key(|r| r.0);
    let mut view = View::new(file)?;
    let mut cursor = start;
    for (at, old_size, new_size, reader) in replacements {
        if at < cursor {
            return Err(bad("overlapping replacement ranges"));
        }
        let next = range(at, old_size, end)?;
        view.append(at - cursor, Content::Disk(cursor))?;
        view.append(new_size, Content::Reader(reader))?;
        cursor = next;
    }
    view.append(end - cursor, Content::Disk(cursor))?;
    Ok(view)
}
