//! Synthetic CI/conformance and read-only local structure audit, not product CLI.
use nxformat_packer::{
    import::{self, structures::NpdmProfile},
    *,
};
use std::{fs, io::Write, path::PathBuf};
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--audit") {
        if args.len() != 3 {
            return Err("--audit TREE 2048|3072".into());
        }
        let profile = match args[2].as_str() {
            "2048" => NpdmProfile::MetaAcid2048V1,
            "3072" => NpdmProfile::MetaAcid3072V1,
            _ => return Err("explicit NPDM profile required".into()),
        };
        let audit = import::audit_executables(
            &PathBuf::from(&args[1]),
            profile,
            &import::ImportLimits::default(),
            &CancellationToken::default(),
        )?;
        println!("APPLICATION unknown; PLATFORM unknown; VERSION unknown; TYPE unknown; DEPENDENCIES unknown; COMPLETENESS unknown");
        if let Some(p) = audit.npdm {
            println!("PROGRAM {:016x}; profile {:?}; flags {}; remote_memory {}; structural only, RSA not authenticated",p.program_id,p.profile,p.flags,p.remote_memory_size);
        }
        for (path, nso) in audit.modules {
            println!(
                "MODULE {path}\t{}\tchecked_segments={}\tdecoded_bytes={}",
                nso.build_id
                    .map(|b| hex(&b))
                    .unwrap_or_else(|| "unknown".into()),
                nso.segment_hashes_checked,
                nso.decoded_bytes
            );
        }
        return Ok(());
    }
    let root = PathBuf::from(args.first().ok_or("OUTPUT [LARGE_MIB]")?);
    fs::create_dir_all(&root)?;
    let mib = args
        .get(1)
        .map(|s| s.parse::<u64>())
        .transpose()?
        .unwrap_or(0);
    let source = root.join("sources");
    let text = import::synthetic::package(
        &source,
        mib.checked_mul(1024 * 1024).ok_or("size overflow")?,
    )?;
    fs::write(root.join("declarations.nximport"), &text)?;
    let limits = import::ImportLimits::default();
    let manifest = import::DeclarationManifest::read(text.as_bytes(), &limits)?;
    let options = WriterOptions {
        max_memory_bytes: 96 * 1024 * 1024,
        ..Default::default()
    };
    let result = import::convert_extracted(
        &source,
        &manifest,
        &root.join("extracted.nx"),
        &limits,
        &options,
    )?;
    fs::write(
        root.join("provenance.nxreceipt"),
        result.receipt.encode(&limits)?,
    )?;
    let archive = inspect_archive(root.join("extracted.nx"), &options, IntegrityPolicy::Strict)?;
    let mut expected = fs::File::create(root.join("expected.tsv"))?;
    let mut selections = fs::File::create(root.join("selection.tsv"))?;
    let mut index = 0;
    for app in &archive.applications {
        for snapshot in &app.snapshots {
            let (key, _) = result
                .receipt
                .snapshots
                .iter()
                .find(|(_, r)| r.identity == snapshot.identity)
                .ok_or("receipt scope missing")?;
            let view = import::inspect_tree(
                &source,
                &manifest,
                key,
                &limits,
                &CancellationToken::default(),
            )?;
            for f in view.files() {
                let (source, offset) = f.host_extent();
                writeln!(
                    expected,
                    "{index}\t{}\t{}\t{offset}\t{}",
                    f.path(),
                    source.display(),
                    f.size()
                )?;
            }
            writeln!(
                selections,
                "{index}\t{:016x}\t{}\t{}\t{}\t{}",
                app.application_id,
                snapshot
                    .program_id
                    .map(|p| format!("{p:016x}"))
                    .unwrap_or_else(|| "-".into()),
                snapshot.numeric_version,
                hex(&snapshot.identity),
                snapshot
                    .build_id
                    .map(|b| hex(&b))
                    .unwrap_or_else(|| "-".into())
            )?;
            index += 1;
        }
    }
    let retained: u64 = result
        .additions
        .iter()
        .map(|s| s.retained_stored_bytes_copied)
        .sum();
    println!("PASS artificial extracted conversion: snapshots={index} compared_bytes={} elapsed_s={:.6} peak_temporary_bytes={} initial_writer_accounted_bytes={} retained_compressed_bytes={} decoded_executable_bytes={}",result.source_bytes_compared,result.elapsed.as_secs_f64(),result.peak_temporary_disk_bytes,result.initial.accounted_memory_bytes,retained,result.decoded_executable_bytes);
    Ok(())
}
