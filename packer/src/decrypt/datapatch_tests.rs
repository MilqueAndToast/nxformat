//! Artificial encryption keys and content only; no console-derived bytes.
use super::tests::Temp;
use super::*;
use crate::native::synthetic::{self, TitleSpec};

#[test]
fn encrypted_complete_datapatch_converts_and_replays_decrypted_recipe() {
    let app = 0x0100000061230000u64;
    let data = app + 0x1001;
    let mut entries = synthetic::title_entries(&TitleSpec::application(app, 0, 1));
    let mut spec = TitleSpec::data_patch(app, data, data + 0x800, 7, 2);
    spec.data_owner = Some(data + 0x800);
    entries.extend(synthetic::title_entries(&spec));
    let content_key = [0x51u8; 16];
    let area_key = [0x49u8; 16];
    let upper = [1, 2, 3, 4, 0, 0, 0, 9];
    for (_, bytes) in &mut entries {
        bytes[0x200..0x204].copy_from_slice(b"NCA3");
        bytes[0x206] = 0;
        bytes[0x220] = 0;
        let mut wrapped = content_key.into();
        Aes128::new((&area_key).into()).encrypt_block(&mut wrapped);
        bytes[0x320..0x330].copy_from_slice(&wrapped);
        for slot in 0..4 {
            let start = u32le(bytes, 0x240 + slot * 16) as usize * 512;
            let end = u32le(bytes, 0x244 + slot * 16) as usize * 512;
            if start == end {
                continue;
            }
            let fs = 0x400 + slot * 512;
            bytes[fs + 4] = 3;
            bytes[fs + 0x140..fs + 0x148].copy_from_slice(&u64::from_be_bytes(upper).to_le_bytes());
            let hash = sha256(&bytes[fs..fs + 512]);
            bytes[0x280 + slot * 32..0x2a0 + slot * 32].copy_from_slice(&hash);
            ctr(
                &mut bytes[start..end],
                &Aes128::new((&content_key).into()),
                upper,
                start as u64,
            );
        }
    }
    let disk = synthetic::dnsp(&entries);
    let input = Temp::new(&disk);
    let mut file = File::open(&input.0).unwrap();
    let keys = Keys {
        values: HashMap::from([
            ("header_key".into(), Zeroizing::new((0..32).collect())),
            (
                "key_area_key_application_00".into(),
                Zeroizing::new(area_key.to_vec()),
            ),
        ]),
    };
    let (size, reader) = package(
        &mut file,
        0,
        disk.len() as u64,
        false,
        false,
        &keys,
        &HashMap::new(),
    )
    .unwrap();
    let source =
        crate::source_file::SourceRegistration::new("datapatch.nsp", size, Box::new(reader));
    let output = Temp(Temp::new(b"").0.with_extension("nx"));
    crate::cli::run(
        vec![
            "pack".into(),
            source.path().to_string_lossy().into_owned(),
            output.0.to_string_lossy().into_owned(),
            "--recipe".into(),
            "--paranoid".into(),
        ],
        None,
        None,
    )
    .unwrap();
    for command in ["verify", "check-recipe"] {
        crate::cli::run(
            vec![command.into(), output.0.to_string_lossy().into_owned()],
            None,
            None,
        )
        .unwrap();
    }
    let archive = crate::inspect_archive(
        &output.0,
        &crate::WriterOptions::default(),
        crate::IntegrityPolicy::Strict,
    )
    .unwrap();
    assert!(archive.applications[0]
        .snapshots
        .iter()
        .any(|s| s.snapshot_type == crate::SnapshotType::Dlc && s.numeric_version == 7));
    assert_eq!(std::fs::read(&input.0).unwrap(), disk);
}
