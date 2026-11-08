//! Optional, bounded VFS observations. This is performance input, never native metadata.
use crate::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ProfileScope {
    pub platform: u8,
    pub application_id: u64,
    pub program_id: Option<u64>,
    pub snapshot_type: u8,
    pub numeric_version: u32,
    pub content_id: [u8; 16],
    pub build_id: [u8; 32],
    pub snapshot_identity: [u8; 32],
}

#[cfg(test)]
mod tests {
    use super::*;
    fn header() -> String {
        format!("NXREAD1\nF\t1\t1\t0100000000010000\t0100000000010000\t0\t0\t{}\t{}\t{}\t100000\t{}\t66696c65\n","00".repeat(16),"00".repeat(32),"01".repeat(32),"02".repeat(32))
    }
    #[test]
    fn trace_threads_keep_independent_sequential_histories() {
        let text=format!("{}R\t1\t1\t1\t1\t0\t4096\t1\t2\nR\t2\t1\t1\t2\t0\t4096\t2\t2\nR\t3\t1\t1\t1\t4096\t4096\t3\t2\nR\t4\t1\t1\t2\t4096\t4096\t4\t2\nZ\t4\t0\n",header());
        let parsed = CapturedReadProfiles::read(text.as_bytes()).unwrap();
        assert_eq!(parsed.files[0].profile.observations(), 4);
        assert_eq!(parsed.files[0].profile.contiguous_reads(), 2);
        for bad in [
            text.replace("Z\t4\t0", "Z\t4\t1"),
            text.replace("Z\t4\t0\n", ""),
            text.replace("Z\t4\t0", "Z\t3\t0"),
            text.clone() + "garbage",
            text.replace("\t100000\t", "\t10\t"),
            text.replace("R\t3\t", "R\t1\t"),
            text.replace("R\t1\t1", "R\t1\t2"),
        ] {
            assert!(CapturedReadProfiles::read(bad.as_bytes()).is_err());
        }
    }
    #[test]
    fn malformed_trace_allocations_are_bounded() {
        assert!(CapturedReadProfiles::read(vec![b'a'; 20000].as_slice()).is_err());
        let bad = format!(
            "{}F\t1\t1\t0100000000010000\t-\t0\t0\t{}\t{}\t{}\t1\t{}\t2e2e\nZ\t0\t0\n",
            header(),
            "00".repeat(16),
            "00".repeat(32),
            "00".repeat(32),
            "00".repeat(32)
        );
        assert!(CapturedReadProfiles::read(bad.as_bytes()).is_err());
    }
}
#[derive(Clone, Debug)]
pub struct CapturedFileProfile {
    pub scope: ProfileScope,
    pub path: String,
    pub size: u64,
    pub sha256: [u8; 32],
    pub profile: FileReadProfile,
}
#[derive(Clone, Debug, Default)]
pub struct CapturedReadProfiles {
    pub files: Vec<CapturedFileProfile>,
}
fn invalid(s: &str) -> WriterError {
    WriterError::new(WriterErrorKind::InvalidInput, s)
}
fn number<T: std::str::FromStr>(s: &str) -> Result<T, WriterError> {
    s.parse().map_err(|_| invalid("invalid trace integer"))
}
pub fn hex<const N: usize>(s: &str) -> Result<[u8; N], WriterError> {
    if s.len() != N * 2 || !s.is_ascii() {
        return Err(invalid("invalid trace hexadecimal field"));
    }
    let mut out = [0; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
            .map_err(|_| invalid("invalid trace hexadecimal field"))?;
    }
    Ok(out)
}
impl ProfileScope {
    pub(crate) fn matches_snapshot(&self, s: &SnapshotInput) -> bool {
        self.program_id == s.program_id
            && self.snapshot_type == s.snapshot_type as u8
            && self.numeric_version == s.numeric_version
            && self.content_id == s.content_id.unwrap_or([0; 16])
            && self.build_id == s.build_id.unwrap_or([0; 32])
    }
}
impl CapturedReadProfiles {
    /// NXREAD1 TSV. At most 64 MiB, 4096 files, 16384 handle/thread streams.
    /// A lossless final Z record is required. Events contain no payload bytes.
    pub fn read<R: Read>(reader: R) -> Result<Self, WriterError> {
        let mut reader = BufReader::new(reader.take(64 * 1024 * 1024 + 1));
        let mut total = 0;
        let mut line = String::new();
        let mut files = BTreeMap::<u64, CapturedFileProfile>::new();
        let mut streams = HashMap::<(u64, u64, u64), (u64, u64)>::new();
        let mut header = false;
        let mut footer = false;
        let mut events = 0u64;
        let mut sequences = HashSet::new();
        loop {
            line.clear();
            // Bound individual allocation even for malicious input without newlines.
            let n = reader.by_ref().take(16385).read_line(&mut line)?;
            if n == 0 {
                break;
            }
            total += n;
            if n > 16384 || total > 64 * 1024 * 1024 || footer {
                return Err(invalid("oversized or trailing trace data"));
            }
            let fields: Vec<_> = line.trim_end_matches(['\r', '\n']).split('\t').collect();
            match fields.as_slice() {
                ["NXREAD1"] if !header => header = true,
                ["F", id, platform, app, program, kind, version, content, build, identity, size, digest, path]
                    if header =>
                {
                    if files.len() >= 4096 || path.len() > 8192 {
                        return Err(invalid("trace file limit"));
                    }
                    let mut bytes = Vec::with_capacity(path.len() / 2);
                    if path.len() % 2 != 0 || !path.is_ascii() {
                        return Err(invalid("invalid trace path"));
                    }
                    for i in (0..path.len()).step_by(2) {
                        bytes.push(
                            u8::from_str_radix(&path[i..i + 2], 16)
                                .map_err(|_| invalid("invalid trace path"))?,
                        );
                    }
                    let path = String::from_utf8(bytes)
                        .map_err(|_| invalid("invalid UTF-8 trace path"))?;
                    if path.is_empty()
                        || path.split('/').any(|p| {
                            p.is_empty() || p == "." || p == ".." || p.contains(['\\', '\0'])
                        })
                    {
                        return Err(invalid("unsafe trace path"));
                    }
                    let scope = ProfileScope {
                        platform: number(platform)?,
                        application_id: u64::from_be_bytes(hex(app)?),
                        program_id: if *program == "-" {
                            None
                        } else {
                            Some(u64::from_be_bytes(hex(program)?))
                        },
                        snapshot_type: number(kind)?,
                        numeric_version: number(version)?,
                        content_id: hex(content)?,
                        build_id: hex(build)?,
                        snapshot_identity: hex(identity)?,
                    };
                    let file = CapturedFileProfile {
                        scope,
                        path,
                        size: number(size)?,
                        sha256: hex(digest)?,
                        profile: FileReadProfile::default(),
                    };
                    if files.insert(number(id)?, file).is_some() {
                        return Err(invalid("duplicate trace file ID"));
                    }
                }
                ["R", sequence, file, handle, thread, offset, bytes, start_ns, latency_ns]
                    if header =>
                {
                    let sequence: u64 = number(sequence)?;
                    if events == 100000 || sequence == 0 || !sequences.insert(sequence) {
                        return Err(invalid("duplicate/excessive trace sequence"));
                    }
                    let file: u64 = number(file)?;
                    let key = (file, number(handle)?, number(thread)?);
                    let offset: u64 = number(offset)?;
                    let bytes: u32 = number(bytes)?;
                    let _: u64 = number(start_ns)?;
                    let _: u64 = number(latency_ns)?;
                    if bytes == 0 {
                        return Err(invalid("zero successful read"));
                    }
                    let profile = &mut files
                        .get_mut(&file)
                        .ok_or_else(|| invalid("trace references unknown file"))?
                        .profile;
                    if streams.len() >= 16384 && !streams.contains_key(&key) {
                        return Err(invalid("trace stream limit"));
                    }
                    // Each handle/thread stream is ordered; unrelated threads do not
                    // turn independent sequential readers into random-access evidence.
                    let previous = streams.get(&key).copied();
                    if previous.is_some_and(|(seq, _)| seq >= sequence) {
                        return Err(invalid("unordered trace stream"));
                    }
                    profile.observe_stream(offset, bytes, previous.map(|(_, end)| end));
                    let end = offset
                        .checked_add(bytes as u64)
                        .ok_or_else(|| invalid("trace read overflow"))?;
                    streams.insert(key, (sequence, end));
                    events += 1;
                }
                ["Z", count, dropped] if header => {
                    if number::<u64>(count)? != events || number::<u64>(dropped)? != 0 {
                        return Err(invalid("incomplete/lossy capture"));
                    }
                    footer = true;
                }
                _ => return Err(invalid("invalid NXREAD1 trace record")),
            }
        }
        if !header || !footer || events == 0 {
            return Err(invalid("missing complete capture footer or observations"));
        }
        let files: Vec<_> = files.into_values().collect();
        for f in &files {
            if !f.profile.fits(f.size) {
                return Err(invalid("trace exceeds file extent"));
            }
        }
        let mut keys = BTreeSet::new();
        for a in &files {
            if !keys.insert((&a.scope, &a.path)) {
                return Err(invalid("ambiguous captured file"));
            }
        }
        Ok(Self { files })
    }
    /// Attach only exact scopes/paths. The writer checks streamed SHA-256 and
    /// the resulting canonical snapshot identity before atomic publication.
    pub fn attach(
        &self,
        input: &mut ArchiveInput,
        require_all: bool,
    ) -> Result<Vec<usize>, WriterError> {
        let mut keys = BTreeSet::new();
        for file in &self.files {
            if !keys.insert((&file.scope, &file.path)) {
                return Err(invalid("ambiguous captured file"));
            }
        }
        let mut matched = Vec::new();
        for (index, profile) in self.files.iter().enumerate() {
            let mut hits = 0;
            for app in &mut input.applications {
                if profile.scope.platform != app.platform as u8
                    || profile.scope.application_id != app.application_id
                {
                    continue;
                }
                for s in &mut app.snapshots {
                    if !profile.scope.matches_snapshot(s) {
                        continue;
                    }
                    let mut dir = &mut s.root;
                    let components: Vec<_> = profile.path.split('/').collect();
                    for (i, part) in components.iter().enumerate() {
                        let Some(entry) = dir.entries.iter_mut().find(|e| e.name == *part) else {
                            return Err(invalid("captured path absent from matching snapshot"));
                        };
                        match &mut entry.node {
                            NodeInput::File(file)
                                if i + 1 == components.len() && file.size == profile.size =>
                            {
                                file.read_profile = profile.profile;
                                file.captured = Some(std::sync::Arc::new(profile.clone()));
                                hits += 1;
                                break;
                            }
                            NodeInput::Directory(child) if i + 1 < components.len() => dir = child,
                            _ => return Err(invalid("captured file structure mismatch")),
                        }
                    }
                }
            }
            if hits > 1 || (require_all && hits != 1) {
                return Err(invalid("missing or ambiguous captured snapshot scope"));
            }
            if hits == 1 {
                matched.push(index);
            }
        }
        Ok(matched)
    }
}
