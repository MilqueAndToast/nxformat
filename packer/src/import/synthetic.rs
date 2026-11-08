//! Artificial, redistributable structure fixtures. NSO bytes contain no code.
use super::structures::NpdmProfile;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn put(b: &mut [u8], at: usize, n: u32) {
    b[at..at + 4].copy_from_slice(&n.to_le_bytes());
}
pub fn npdm(program: u64, profile: NpdmProfile) -> Vec<u8> {
    let magic = match profile {
        NpdmProfile::MetaAcid2048V1 => 0x200,
        NpdmProfile::MetaAcid3072V1 => 0x300,
        NpdmProfile::Absent => panic!("fixture requires NPDM profile"),
    };
    let acid_len = magic + 0x40 + 0x2c;
    let aci_at = 0x80 + acid_len;
    let mut b = vec![0; aci_at + 0x40 + 0x1c];
    b[..4].copy_from_slice(b"META");
    b[0xc] = 0x27;
    b[0xe] = 0x20;
    put(&mut b, 0x1c, 0x4000);
    put(&mut b, 0x70, aci_at as u32);
    put(&mut b, 0x74, 0x40 + 0x1c);
    put(&mut b, 0x78, 0x80);
    put(&mut b, 0x7c, acid_len as u32);
    b[0x80 + magic..0x84 + magic].copy_from_slice(b"ACID");
    put(&mut b, 0x84 + magic, (acid_len - magic / 2) as u32);
    b[0x90 + magic..0x98 + magic].copy_from_slice(&program.to_le_bytes());
    b[0x98 + magic..0xa0 + magic].copy_from_slice(&program.to_le_bytes());
    b[aci_at..aci_at + 4].copy_from_slice(b"ACI0");
    b[aci_at + 0x10..aci_at + 0x18].copy_from_slice(&program.to_le_bytes());
    put(&mut b, aci_at + 0x20, 0x40);
    put(&mut b, aci_at + 0x24, 0x1c);
    b[aci_at + 0x40] = 1;
    put(&mut b, 0x80 + magic + 0x20, (magic + 0x40) as u32);
    put(&mut b, 0x80 + magic + 0x24, 0x2c);
    b[0x80 + magic + 0x40] = 1;
    b
}
fn repeat_lz4(value: u8, size: usize) -> Vec<u8> {
    assert!(size >= 32);
    let mut out = vec![0x1f, value, 1, 0];
    let mut extra = size - 28;
    while extra >= 255 {
        out.push(255);
        extra -= 255;
    }
    out.push(extra as u8);
    out.push(0x80);
    out.extend_from_slice(&[value; 8]);
    out
}
pub fn nso(build: u8, compressed: bool) -> Vec<u8> {
    let payloads = [vec![build; 131072], vec![0x55; 64], vec![0xaa; 32]];
    let mut b = vec![0; 0x100];
    b[..4].copy_from_slice(b"NSO0");
    b[0x40..0x60].fill(build);
    put(&mut b, 0xc, 0x38 + u32::from(compressed));
    for (i, p) in payloads.iter().enumerate() {
        let stored = if i == 0 && compressed {
            repeat_lz4(build, p.len())
        } else {
            p.clone()
        };
        let at = b.len() as u32;
        put(&mut b, 0x10 + i * 16, at);
        put(&mut b, 0x14 + i * 16, [0, 0x20000, 0x21000][i]);
        put(&mut b, 0x18 + i * 16, p.len() as u32);
        put(&mut b, 0x60 + i * 4, stored.len() as u32);
        b[0xa0 + i * 32..0xc0 + i * 32].copy_from_slice(&crate::hash::sha256(p));
        b.extend(stored);
    }
    b
}
/// An NSO whose three segments are ZBIC-compressed (flags 0xff), as in
/// Switch 2 games. The payloads are the magic and filler, not real ZBIC.
pub fn nso_zbic(build: u8) -> Vec<u8> {
    let mut b = vec![0; 0x100];
    b[..4].copy_from_slice(b"NSO0");
    b[0x40..0x60].fill(build);
    put(&mut b, 0xc, 0xff);
    for (i, logical) in [131072u32, 64, 32].into_iter().enumerate() {
        let mut stored = b"ZBIC".to_vec();
        stored.extend([build; 12]);
        let at = b.len() as u32;
        put(&mut b, 0x10 + i * 16, at);
        put(&mut b, 0x14 + i * 16, [0, 0x20000, 0x21000][i]);
        put(&mut b, 0x18 + i * 16, logical);
        put(&mut b, 0x60 + i * 4, stored.len() as u32);
        // A segment hash that cannot match: ZBIC segments are not decoded.
        b[0xa0 + i * 32..0xc0 + i * 32].fill(0xee);
        b.extend(stored);
    }
    b
}
pub fn executable_tree(root: &Path, program: u64, build: u8) -> io::Result<()> {
    fs::create_dir_all(root.join("exefs"))?;
    fs::create_dir_all(root.join("romfs/empty"))?;
    fs::write(
        root.join("exefs/main.npdm"),
        npdm(program, NpdmProfile::MetaAcid3072V1),
    )?;
    fs::write(root.join("exefs/main"), nso(build, true))?;
    fs::write(root.join("exefs/sdk"), nso(0x60, false))?;
    fs::write(root.join("romfs/data.bin"), b"artificial common data")?;
    fs::write(root.join("romfs/delete.me"), b"explicit tombstone target")?;
    Ok(())
}
pub fn row(
    key: &str,
    root: &str,
    kind: &str,
    version: u32,
    complete: bool,
    program: u64,
) -> String {
    format!("snapshot\t{key}\t{root}\tswitch2\t0000000000001234\t{program:016x}\tprimary\t{kind}\t{version}\t{}\tmaterialized-v1\tmeta-acid3072-v1\n",if complete{"complete"}else{"partial"})
}
pub fn package(root: &Path, large_bytes: u64) -> io::Result<String> {
    executable_tree(&root.join("base"), 0x5678, 0x41)?;
    executable_tree(&root.join("updates/not-a-version"), 0x5678, 0x42)?;
    executable_tree(&root.join("updates/new-executables"), 0x5678, 0x43)?;
    fs::write(
        root.join("updates/new-executables/exefs/sdk"),
        nso(0x61, true),
    )?;
    fs::create_dir_all(root.join("dlc/not-a-title/7/data"))?;
    fs::write(
        root.join("dlc/not-a-title/7/data/content.bin"),
        b"artificial DLC",
    )?;
    if large_bytes != 0 {
        let mut out = fs::File::create(root.join("base/romfs/large.bin"))?;
        let chunk = [0x35; 65536];
        let mut left = large_bytes;
        while left != 0 {
            let n = left.min(chunk.len() as u64) as usize;
            out.write_all(&chunk[..n])?;
            left -= n as u64;
        }
    }
    let mut text = "NXIMPORT\t1\n".to_owned();
    text.push_str(&row("b", "base", "base", 0, true, 0x5678));
    text.push_str(&row(
        "u",
        "updates/not-a-version",
        "update",
        10,
        true,
        0x5678,
    ));
    text.push_str("dependency\tu\trequired-base\tb\ntitle\tu\tDeclared title, never native display metadata\n");
    text.push_str(&row(
        "v",
        "updates/new-executables",
        "update",
        20,
        true,
        0x5678,
    ));
    text.push_str("dependency\tv\trequired-base\tb\n");
    text.push_str("snapshot\td\tdlc/not-a-title/7\tswitch2\t0000000000001234\t-\tnone\tdlc\t1\tcomplete\tmaterialized-v1\tnone\ncontent\td\t11111111111111111111111111111111\ndependency\td\trequires\tv\n");
    Ok(text)
}
