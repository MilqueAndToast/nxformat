//! Review fixtures and verification measurements, not a user CLI.
use nxformat_packer::native::{self, InventoryMode, InventoryOptions};
use nxformat_packer::*;
use std::{fs, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args[0].as_str() {
        "verify" => {
            let options = WriterOptions {
                strict_file_cache_entries: args[2].parse()?,
                max_memory_bytes: args[3].parse::<u64>()? * 1048576,
                compression_mode: CompressionMode::RawOnly,
                ..WriterOptions::default()
            };
            let stats = repack_archive(
                &args[1],
                &args[1],
                RepackPlan::new(),
                &options,
                IntegrityPolicy::Strict,
            )?;
            println!("{stats:?}");
            println!(
                "VERIFY,{:.9},{},{},{},{},{},{},{}",
                stats.elapsed.as_secs_f64(),
                stats.source_verification.files_verified,
                stats.source_verification.files_reused,
                stats.source_verification.logical_file_bytes_verified,
                stats.source_verification.logical_file_bytes_reused,
                stats.source_verification.block_decodes,
                stats.source_verification.decompressed_bytes,
                stats.source_verification.cache_peak_entries
            );
            println!(
                "METRIC,verify,{:.9},{},0,0,0,0,0,0,0,0,{},0",
                stats.elapsed.as_secs_f64(),
                stats.archive_bytes,
                stats.source_payload_bytes_verified
            );
        }
        "convert" => {
            let options = InventoryOptions {
                mode: InventoryMode::Strict,
                ..InventoryOptions::default()
            };
            let paths: Vec<_> = args[2..].iter().map(PathBuf::from).collect();
            let inv = native::inventory(&paths, &options)?;
            for issue in inv.all_issues() {
                if issue.severity == native::Severity::Error {
                    eprintln!("{issue:?}");
                }
            }
            let stats = conversion::convert_inventory(
                &inv,
                std::path::Path::new(&args[1]),
                &options,
                IntegrityPolicy::Strict,
                &WriterOptions::default(),
            )?;
            println!("{stats:?}");
        }
        "fixtures" => {
            use native::synthetic::*;
            let root = PathBuf::from(&args[1]);
            fs::create_dir_all(&root)?;
            const APP: u64 = 0x0100000000010000;
            let mut base = TitleSpec::application(APP, 0, 1);
            base.program.as_mut().unwrap().romfs_bytes = 16 * 1048576;
            let mut second = base.program.clone().unwrap();
            second.program_id = APP + 1;
            second.build_id = [7; 32];
            base.extra_programs.push(second);
            base.control.as_mut().unwrap().title = "First game".into();
            let mut second_control = TitleSpec::application(APP, 0, 55);
            second_control.program.as_mut().unwrap().program_id = APP + 1;
            second_control.control.as_mut().unwrap().title = "Second game".into();
            let control = title_entries(&second_control)
                .into_iter()
                .find(|(_, b)| b[0x205] == 2)
                .unwrap();
            let original = title_entries_custom(&base, &[0, 1, 0, 1], &[(3, control.0, control.1)]);
            fs::write(root.join("base.nsp"), dnsp(&original))?;
            for version in 1..=4 {
                let mut p = TitleSpec::patch(APP, version, 20 + version as u8);
                p.control = None;
                if version == 1 {
                    p.program.as_mut().unwrap().program_id = APP + 1;
                }
                if version == 2 {
                    let mut extra = p.program.clone().unwrap();
                    extra.program_id = APP + 1;
                    extra.build_id = [99; 32];
                    p.extra_programs.push(extra);
                }
                if version == 3 {
                    p.program = None;
                    p.data = Some(vec![5; 1234567]);
                }
                let shared = if version == 4 {
                    let original = original.iter().find(|(_, data)| data[0x205] == 0).unwrap();
                    vec![(0, original.0.clone(), original.1.clone())]
                } else {
                    Vec::new()
                };
                fs::write(
                    root.join(format!("update-{version}.nsp")),
                    dnsp(&title_entries_custom(
                        &p,
                        if version == 1 { &[1] } else { &[] },
                        &shared,
                    )),
                )?;
            }
            for i in 1..=2 {
                let mut p = TitleSpec::add_on(APP, APP + 0x1000 + i, 0, 70 + i as u8);
                if i == 2 {
                    p.required_application_version = 2;
                }
                fs::write(root.join(format!("dlc-{i}.nsp")), dnsp(&title_entries(&p)))?;
            }
        }
        _ => return Err("review harness mode absent".into()),
    }
    Ok(())
}
