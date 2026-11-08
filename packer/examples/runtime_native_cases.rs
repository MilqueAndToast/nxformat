//! Native association fixtures for the real NXEmu NCA/VFS path, never games.
use nxformat_packer::native::{self, synthetic::*, InventoryOptions};
use nxformat_packer::*;
use std::{fs, path::PathBuf};

const APP: u64 = 0x0100000000010000;
const AUX: u64 = APP + 0x123;
fn npdm(id: u64) -> Vec<u8> {
    let mut b = vec![0; 0x400];
    b[..4].copy_from_slice(b"META");
    b[0x70..0x74].copy_from_slice(&0x80u32.to_le_bytes());
    b[0x74..0x78].copy_from_slice(&0x80u32.to_le_bytes());
    b[0x78..0x7c].copy_from_slice(&0x100u32.to_le_bytes());
    b[0x7c..0x80].copy_from_slice(&0x300u32.to_le_bytes());
    b[0x80..0x84].copy_from_slice(b"ACI0");
    b[0x90..0x98].copy_from_slice(&id.to_le_bytes());
    b[0xa0..0xa4].copy_from_slice(&0x40u32.to_le_bytes());
    b[0xa4..0xa8].copy_from_slice(&0x1cu32.to_le_bytes());
    b[0x300..0x304].copy_from_slice(b"ACID");
    b[0x310..0x318].copy_from_slice(&id.to_le_bytes());
    b[0x318..0x320].copy_from_slice(&id.to_le_bytes());
    b[0x320..0x324].copy_from_slice(&0x240u32.to_le_bytes());
    b[0x324..0x328].copy_from_slice(&0x2cu32.to_le_bytes());
    b
}
fn prepare(p: &mut ProgramSpec) {
    p.bktr = false; // Complete replacement, not a fabricated valid BKTR table.
    p.npdm = Some(npdm(p.program_id));
    p.romfs_bytes = 800_000;
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(std::env::args().nth(1).ok_or("output directory")?);
    let sources = root.join("sources");
    fs::create_dir_all(&sources)?;
    let mut base = TitleSpec::application(APP, 0, 1);
    prepare(base.program.as_mut().unwrap());
    let mut aux = base.program.clone().unwrap();
    aux.program_id = AUX;
    aux.build_id = [7; 32];
    prepare(&mut aux);
    base.extra_programs.push(aux);
    fs::write(
        sources.join("base.nsp"),
        dnsp(&title_entries_custom(&base, &[0, 7, 0], &[])),
    )?;
    for version in 1..=3 {
        let mut update = TitleSpec::patch(APP, version, 20 + version as u8);
        update.control = None;
        if version == 1 {
            update.program.as_mut().unwrap().program_id = AUX;
            // Metadata deliberately belongs to the auxiliary version only.
            update.control = TitleSpec::application(APP, 0, 1).control;
            update.control.as_mut().unwrap().title = "Auxiliary display metadata".into();
        }
        if version == 2 {
            let mut extra = update.program.clone().unwrap();
            extra.program_id = AUX;
            extra.build_id = [99; 32];
            update.extra_programs.push(extra);
        }
        if version == 3 {
            update.program = None;
            update.data = Some(vec![5; 900_000]);
        }
        for p in update.program.iter_mut().chain(&mut update.extra_programs) {
            prepare(p);
        }
        fs::write(
            sources.join(format!("update-{version}.nsp")),
            dnsp(&title_entries_custom(
                &update,
                if version == 1 {
                    &[7, 7]
                } else if version == 2 {
                    &[0, 7]
                } else {
                    &[]
                },
                &[],
            )),
        )?;
    }
    for index in 1..=2 {
        let mut dlc = TitleSpec::add_on(APP, APP + 0x1000 + index, 0, 70 + index as u8);
        if index == 2 {
            dlc.required_application_version = 2;
        }
        fs::write(
            sources.join(format!("dlc-{index}.nsp")),
            dnsp(&title_entries(&dlc)),
        )?;
    }
    let mut paths: Vec<_> = fs::read_dir(&sources)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()?;
    paths.sort();
    let options = InventoryOptions::default();
    let inventory = native::inventory(&paths, &options)?;
    let destination = root.join("native.nx");
    conversion::convert_inventory(
        &inventory,
        &destination,
        &options,
        IntegrityPolicy::Strict,
        &WriterOptions::default(),
    )?;
    let inspected = inspect_archive(
        &destination,
        &WriterOptions::default(),
        IntegrityPolicy::Strict,
    )?;
    let application = &inspected.applications[0];
    let metadata = application
        .snapshots
        .iter()
        .find(|s| s.program_id == Some(AUX) && s.numeric_version == 1)
        .ok_or("auxiliary metadata snapshot")?;
    let mut plan = RepackPlan::new();
    plan.metadata_sources.push(MetadataSourceSelection {
        platform: Platform::Switch1,
        application_id: APP,
        snapshot_identity: metadata.identity,
    });
    repack_archive(
        &destination,
        &destination,
        plan,
        &WriterOptions::default(),
        IntegrityPolicy::Strict,
    )?;
    // A read-only NX may preserve references to absent provenance snapshots;
    // the runtime must additionally require its native base to be available.
    let planned = inventory.plan();
    let absent = planned.applications[0].to_application_input(
        &|s| s.snapshot_type == SnapshotType::Update && s.numeric_version == 3,
        &|r| {
            application
                .snapshots
                .iter()
                .find(|s| {
                    s.program_id == r.program_id
                        && s.numeric_version == r.numeric_version
                        && s.snapshot_type == r.snapshot_type
                })
                .map(|s| s.identity)
        },
    )?;
    write_archive(
        ArchiveInput::new(vec![absent]),
        root.join("missing-base.nx"),
        &WriterOptions::default(),
    )?;
    println!("Generated artificial native associations; NSOs contain no executable code.");
    Ok(())
}
