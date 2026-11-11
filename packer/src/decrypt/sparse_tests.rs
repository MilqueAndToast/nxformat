//! Artificial fixtures only: no console keys or copyrighted game data.
use super::tests::Temp;
use super::*;
use crate::native::synthetic;

const KEY: [u8; 16] = [0x53; 16];
const UPPER: [u8; 8] = [1, 2, 3, 4, 0, 0, 0, 9];
const LOGICAL: usize = 0x20000;
const TABLE: usize = 0x1c00;
fn p32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn p64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}
fn keys() -> Keys {
    Keys {
        values: HashMap::from([
            ("header_key".into(), Zeroizing::new((0..32).collect())),
            (
                "key_area_key_application_00".into(),
                Zeroizing::new(vec![0x49; 16]),
            ),
        ]),
    }
}
fn header_hash(b: &mut [u8]) {
    let h = sha256(&b[0x400..0x600]);
    b[0x280..0x2a0].copy_from_slice(&h);
}
fn table_cipher(b: &mut [u8]) {
    ctr(
        &mut b[TABLE..],
        &Aes128::new((&KEY).into()),
        [1, 2, 3, 4, 0, 5, 0, 0],
        TABLE as u64,
    );
}
fn entry(size: usize) -> Entry {
    Entry {
        start: 0,
        size: size as u64,
        name: "synthetic.nca".into(),
        table: 0,
        data: 0,
    }
}
fn fixture(enc: u8) -> (Vec<u8>, Vec<u8>) {
    let mut b = vec![0; TABLE + 0x8000];
    b[0x200..0x204].copy_from_slice(b"NCA3");
    let len = b.len() as u64;
    p64(&mut b, 0x208, len);
    p32(&mut b, 0x240, 6);
    p32(&mut b, 0x244, (0xc00 + LOGICAL) as u32 / 512);
    let mut wrapped = KEY.into();
    Aes128::new((&[0x49; 16]).into()).encrypt_block(&mut wrapped);
    b[0x320..0x330].copy_from_slice(&wrapped);
    let fs = &mut b[0x400..0x600];
    fs[0] = 2;
    fs[3] = 1;
    fs[4] = enc;
    fs[0x140..0x148].copy_from_slice(&u64::from_be_bytes(UPPER).to_le_bytes());
    if enc >= 5 {
        fs[3] = 2;
        p32(fs, 0x2c, 1);
        p64(fs, 0x30, 0x200);
    }
    p64(fs, 0x148, 0x1000);
    p64(fs, 0x150, 0x8000);
    fs[0x158..0x15c].copy_from_slice(b"BKTR");
    p32(fs, 0x15c, 1);
    p32(fs, 0x160, 5);
    p64(fs, 0x168, 0xc00);
    fs[0x170] = 5;
    let mappings = [
        (0, 0, 1),
        (0x100, 0x600, 0),
        (0x500, 0, 1),
        (0x600, 0, 0),
        (0xa00, 0, 1),
    ];
    let t = &mut b[TABLE..];
    p32(t, 4, 1);
    p64(t, 8, LOGICAL as u64);
    p32(t, 0x4004, 5);
    p64(t, 0x4008, LOGICAL as u64);
    for (i, (v, p, s)) in mappings.into_iter().enumerate() {
        let at = 0x4010 + i * 20;
        p64(t, at, v);
        p64(t, at + 8, p);
        p32(t, at + 16, s);
    }
    let mut expected = vec![0; LOGICAL];
    for (start, physical) in [(0x100, 0x600), (0x600, 0)] {
        for i in 0..0x400 {
            expected[start + i] = (i as u8).wrapping_mul(17).wrapping_add((start / 256) as u8);
        }
        let mut encrypted = expected[start..start + 0x400].to_vec();
        if enc != 1 {
            let skip = if enc >= 5 {
                0x200usize.saturating_sub(start).min(0x400)
            } else {
                0
            };
            ctr(
                &mut encrypted[skip..],
                &Aes128::new((&KEY).into()),
                UPPER,
                (0xc00 + start + skip) as u64,
            );
        }
        b[0xc00 + physical..0x1000 + physical].copy_from_slice(&encrypted);
    }
    table_cipher(&mut b);
    header_hash(&mut b);
    (b, expected)
}
fn decode(b: &[u8]) -> io::Result<(u64, Decrypted)> {
    let temp = Temp::new(b);
    nca(
        &mut File::open(&temp.0)?,
        &entry(b.len()),
        &keys(),
        &HashMap::new(),
    )
}

#[test]
fn sparse_content_uses_logical_counters_and_zeroes_holes_in_every_ctr_mode() {
    for enc in [1, 3, 4, 5, 6] {
        let (b, expected) = fixture(enc);
        let (size, mut reader) = decode(&b).unwrap();
        assert_eq!(size, (0xc00 + LOGICAL) as u64);
        let mut actual = vec![0; LOGICAL];
        for (i, chunk) in actual.chunks_mut(137).enumerate() {
            reader.read_at(0xc00 + (i * 137) as u64, chunk).unwrap();
        }
        assert_eq!(actual, expected, "encryption type {enc}");
        let mut header = vec![0; 0xc00];
        reader.read_at(0, &mut header).unwrap();
        assert_eq!(&header[0x200..0x204], b"DNCA");
        assert_eq!(u64le(&header, 0x208), size);
        assert_eq!(header[0x404], 1);
        assert_eq!(&header[0x548..0x578], &[0; 0x30]);
        assert_eq!(&header[0x5d0..0x5d4], b"DSPF");
        assert_eq!(sha256(&header[0x400..0x600]), header[0x280..0x2a0]);
        assert!(reader.read_at(size - 1, &mut [0; 2]).is_err());
        assert!(reader.read_at(u64::MAX, &mut [0; 1]).is_err());
    }
}

#[test]
fn sparse_empty_table_is_a_bounded_all_hole_section() {
    let (mut b, _) = fixture(3);
    p32(&mut b, 0x560, 0);
    p64(&mut b, 0x548, 0);
    p64(&mut b, 0x550, 0);
    b.truncate(0xc00);
    p64(&mut b, 0x208, 0xc00);
    // Sparse metadata hashes disappear, but unrelated reserved bytes do not.
    b[0x405] = 1;
    b[0x5a0..0x5d0].fill(0x37);
    header_hash(&mut b);
    let (size, mut reader) = decode(&b).unwrap();
    let mut tail = [0xff; 513];
    reader.read_at(size - 513, &mut tail).unwrap();
    assert_eq!(tail, [0; 513]);
    let mut header = [0; 0xc00];
    reader.read_at(0, &mut header).unwrap();
    assert_eq!(header[0x405], 0);
    assert_eq!(&header[0x5a0..0x5d0], &[0; 0x30]);
}

#[test]
fn sparse_ctr_ex_reads_patch_metadata_through_the_mapping() {
    for enc in [4, 6] {
        let (mut b, _) = fixture(enc);
        b.truncate(0xc00);
        let mut plain = vec![0x36; 0x10000];
        plain[0x8000..].fill(0);
        p32(&mut plain, 0x8004, 1);
        p64(&mut plain, 0x8008, 0x4000);
        p32(&mut plain, 0xc004, 1);
        p64(&mut plain, 0xc008, 0x4000);
        p32(&mut plain, 0xc01c, 7);
        let mut encrypted = plain.clone();
        let skip = if enc == 6 { 0x200 } else { 0 };
        ctr(
            &mut encrypted[skip..0x4000],
            &Aes128::new((&KEY).into()),
            [1, 2, 3, 4, 0, 0, 0, 7],
            (0xc00 + skip) as u64,
        );
        ctr(
            &mut encrypted[0x4000..],
            &Aes128::new((&KEY).into()),
            UPPER,
            0x4c00,
        );
        b.extend(encrypted);
        let table_at = b.len();
        b.resize(table_at + 0x8000, 0);
        let t = &mut b[table_at..];
        p32(t, 4, 1);
        p64(t, 8, LOGICAL as u64);
        p32(t, 0x4004, 2);
        p64(t, 0x4008, LOGICAL as u64);
        p64(t, 0x4024, 0x10000);
        p32(t, 0x4034, 1);
        ctr(
            t,
            &Aes128::new((&KEY).into()),
            [1, 2, 3, 4, 0, 5, 0, 0],
            table_at as u64,
        );
        let fs = &mut b[0x400..0x600];
        p64(fs, 0x100, 0x4000);
        p64(fs, 0x108, 0x4000);
        p64(fs, 0x120, 0x8000);
        p64(fs, 0x128, 0x8000);
        fs[0x130..0x134].copy_from_slice(b"BKTR");
        p32(fs, 0x134, 1);
        p32(fs, 0x138, 1);
        p64(fs, 0x148, 0x10000);
        p32(fs, 0x160, 2);
        let len = b.len() as u64;
        p64(&mut b, 0x208, len);
        header_hash(&mut b);
        let (_, mut reader) = decode(&b).unwrap();
        let mut actual = vec![0; LOGICAL];
        for (i, chunk) in actual.chunks_mut(509).enumerate() {
            reader.read_at(0xc00 + (i * 509) as u64, chunk).unwrap();
        }
        assert_eq!(&actual[..0x10000], plain);
        assert!(actual[0x10000..].iter().all(|v| *v == 0));
    }
}

#[test]
fn sparse_malformed_tables_fail_without_panics_or_unbounded_reads() {
    for case in 0..15 {
        let (mut b, _) = fixture(3);
        table_cipher(&mut b);
        match case {
            0 => p32(&mut b, 0x560, 1_000_001),
            1 => p64(&mut b, 0x550, 65 * 1024 * 1024),
            2 => p64(&mut b, 0x568, u64::MAX),
            3 => p64(&mut b, 0x548, u64::MAX),
            4 => p32(&mut b, TABLE + 4, 2),
            5 => p64(&mut b, TABLE + 8, LOGICAL as u64 - 1),
            6 => p64(&mut b, TABLE + 0x4010, 1),
            7 => p64(&mut b, TABLE + 0x4010 + 40, 0x100),
            8 => p32(&mut b, TABLE + 0x4010 + 20 + 16, 2),
            9 => p64(&mut b, TABLE + 0x4010 + 20 + 8, 0xfff),
            10 => p64(&mut b, TABLE + 0x4008, LOGICAL as u64 - 1),
            11 => p32(&mut b, TABLE + 0x4004, 6),
            12 => b[0x570] = 0,
            13 => b[0x5d0] = 1,
            _ => p64(&mut b, 0x550, 0x4000),
        }
        table_cipher(&mut b);
        header_hash(&mut b);
        assert!(decode(&b).is_err(), "malformed case {case}");
    }
}

#[test]
fn sparse_growth_updates_nsp_and_nested_hfs_offsets_and_keeps_neighbors() {
    for hfs in [false, true] {
        let (b, expected) = fixture(3);
        let files = vec![
            ("before.bin".into(), vec![0xab; 31]),
            ("sparse.nca".into(), b),
            ("after.bin".into(), vec![0xcd; 67]),
        ];
        let inner = synthetic::partition(&files, hfs);
        let disk = if hfs {
            synthetic::partition(
                &[("secure".into(), inner), ("update".into(), vec![0xef; 101])],
                true,
            )
        } else {
            inner
        };
        let temp = Temp::new(&disk);
        let mut f = File::open(&temp.0).unwrap();
        let (size, mut reader) = package(
            &mut f,
            0,
            disk.len() as u64,
            hfs,
            hfs,
            &keys(),
            &HashMap::new(),
        )
        .unwrap();
        let mut actual = vec![0; size as usize];
        reader.read_at(0, &mut actual).unwrap();
        let out = Temp::new(&actual);
        let mut f = File::open(&out.0).unwrap();
        let (start, limit) = if hfs {
            let root = partition(&mut f, 0, size, true).unwrap();
            assert_eq!(
                bytes(&mut f, root[1].start, root[1].size as usize).unwrap(),
                vec![0xef; 101]
            );
            (root[0].start, root[0].size)
        } else {
            (0, size)
        };
        let entries = partition(&mut f, start, limit, hfs).unwrap();
        assert_eq!(bytes(&mut f, entries[0].start, 31).unwrap(), vec![0xab; 31]);
        assert_eq!(bytes(&mut f, entries[2].start, 67).unwrap(), vec![0xcd; 67]);
        assert_eq!(entries[1].size, (0xc00 + LOGICAL) as u64);
        assert_eq!(
            bytes(&mut f, entries[1].start + 0xc00, LOGICAL).unwrap(),
            expected
        );
        assert_eq!(std::fs::read(&temp.0).unwrap(), disk);
    }
}

#[test]
fn sparse_multi_set_boundaries_and_shrinking_container() {
    let (mut b, _) = fixture(3);
    b.resize(TABLE + 0xc000, 0);
    b[TABLE..].fill(0);
    let logical = 819 * 512u64;
    p32(&mut b, 0x244, (0xc00 + logical) as u32 / 512);
    p64(&mut b, 0x550, 0xc000);
    p32(&mut b, 0x560, 819);
    let len = b.len() as u64;
    p64(&mut b, 0x208, len);
    let t = &mut b[TABLE..];
    p32(t, 4, 2);
    p64(t, 8, logical);
    p64(t, 24, 818 * 512);
    for block in 0..2 {
        let p = (block + 1) * 0x4000;
        let n = if block == 0 { 818 } else { 1 };
        p32(t, p, block as u32);
        p32(t, p + 4, n as u32);
        p64(t, p + 8, if block == 0 { 818 * 512 } else { logical });
        for i in 0..n {
            let q = p + 16 + i * 20;
            p64(t, q, ((block * 818 + i) * 512) as u64);
            p32(t, q + 16, 1);
        }
    }
    table_cipher(&mut b);
    header_hash(&mut b);
    let (_, mut reader) = decode(&b).unwrap();
    let mut boundary = [0xff; 2048];
    reader
        .read_at(0xc00 + 818 * 512 - 511, &mut boundary[..1023])
        .unwrap();
    assert!(boundary[..1023].iter().all(|v| *v == 0));

    // A dense view can also be smaller than the stored sparse table. Subsequent
    // package entries must move backward safely, without unsigned underflow.
    let (mut b, expected) = fixture(3);
    table_cipher(&mut b);
    p32(&mut b, 0x244, 0x1c00 / 512);
    p64(&mut b, TABLE + 8, 0x1000);
    p64(&mut b, TABLE + 0x4008, 0x1000);
    table_cipher(&mut b);
    header_hash(&mut b);
    let disk = synthetic::dnsp(&[
        ("sparse.nca".into(), b),
        ("after.bin".into(), vec![0x67; 80]),
    ]);
    let temp = Temp::new(&disk);
    let mut f = File::open(&temp.0).unwrap();
    let (size, mut reader) = package(
        &mut f,
        0,
        disk.len() as u64,
        false,
        false,
        &keys(),
        &HashMap::new(),
    )
    .unwrap();
    assert!(size < disk.len() as u64);
    let mut data = vec![0; size as usize];
    reader.read_at(0, &mut data).unwrap();
    let tmp = Temp::new(&data);
    let mut f = File::open(&tmp.0).unwrap();
    let entries = partition(&mut f, 0, size, false).unwrap();
    assert_eq!(
        bytes(&mut f, entries[0].start + 0xc00, 0x1000).unwrap(),
        expected[..0x1000]
    );
    assert_eq!(bytes(&mut f, entries[1].start, 80).unwrap(), vec![0x67; 80]);
}

#[test]
fn sparse_base_and_matching_update_convert_verify_and_replay_recipe() {
    let app = 0x0100_0000_1234_0000;
    let mut entries = synthetic::title_entries(&synthetic::TitleSpec::application(app, 0, 1));
    let (_, program) = entries
        .iter_mut()
        .find(|(name, b)| !name.ends_with(".cnmt.nca") && b[0x205] == 0)
        .unwrap();
    program[0x200..0x204].copy_from_slice(b"NCA3");
    program[0x206] = 0;
    program[0x220] = 0;
    let fs = &mut program[0x400..0x600];
    fs[0x158..0x15c].copy_from_slice(b"BKTR");
    p32(fs, 0x15c, 1);
    fs[0x170] = 1;
    header_hash(program);
    // The second section has a real encrypted sparse mapping table. Its data
    // is present, while the executable above is supplied by the update.
    let section_start = u32le(program, 0x250) as usize * 512;
    let section_end = u32le(program, 0x254) as usize * 512;
    assert!(section_end > section_start);
    let table_start = program.len();
    let section_size = (section_end - section_start) as u64;
    program.resize(table_start + 0x8000, 0);
    let table = &mut program[table_start..];
    p32(table, 4, 1);
    p64(table, 8, section_size);
    p32(table, 0x4004, 1);
    p64(table, 0x4008, section_size);
    ctr(
        table,
        &Aes128::new((&KEY).into()),
        [0, 0, 0, 0, 0, 1, 0, 0],
        table_start as u64,
    );
    let fs = &mut program[0x600..0x800];
    fs[0x140..0x148].fill(0);
    p64(fs, 0x148, (table_start - section_start) as u64);
    p64(fs, 0x150, 0x8000);
    fs[0x158..0x15c].copy_from_slice(b"BKTR");
    p32(fs, 0x15c, 1);
    p32(fs, 0x160, 1);
    p64(fs, 0x168, section_start as u64);
    fs[0x170] = 1;
    let digest = sha256(fs);
    program[0x2a0..0x2c0].copy_from_slice(&digest);
    let mut wrapped = KEY.into();
    Aes128::new((&[0x49; 16]).into()).encrypt_block(&mut wrapped);
    program[0x320..0x330].copy_from_slice(&wrapped);
    let len = program.len() as u64;
    p64(program, 0x208, len);
    let disk = synthetic::dnsp(&entries);
    let temp = Temp::new(&disk);
    let mut f = File::open(&temp.0).unwrap();
    let (size, reader) = package(
        &mut f,
        0,
        disk.len() as u64,
        false,
        false,
        &keys(),
        &HashMap::new(),
    )
    .unwrap();
    let base =
        crate::source_file::SourceRegistration::new("sparse-base.nsp", size, Box::new(reader));
    let disk = synthetic::dnsp(&synthetic::title_entries(&synthetic::TitleSpec::patch(
        app, 65536, 2,
    )));
    let update_temp = Temp::new(&disk);
    let mut f = File::open(&update_temp.0).unwrap();
    let (size, reader) = package(
        &mut f,
        0,
        disk.len() as u64,
        false,
        false,
        &keys(),
        &HashMap::new(),
    )
    .unwrap();
    let update = crate::source_file::SourceRegistration::new("update.nsp", size, Box::new(reader));
    let output = Temp(Temp::new(b"").0.with_extension("nx"));
    crate::cli::run(
        vec![
            "pack".into(),
            base.path().to_string_lossy().into(),
            update.path().to_string_lossy().into(),
            output.0.to_string_lossy().into(),
            "--recipe".into(),
            "--paranoid".into(),
            "--level".into(),
            "1".into(),
            "--workers".into(),
            "1".into(),
        ],
        None,
        None,
    )
    .unwrap();
    for cmd in ["verify", "check-recipe"] {
        crate::cli::run(
            vec![cmd.into(), output.0.to_string_lossy().into()],
            None,
            None,
        )
        .unwrap();
    }
}
