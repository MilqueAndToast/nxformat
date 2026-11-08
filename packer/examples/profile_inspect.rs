//! Validate a bounded, lossless NXEmu trace without reading proprietary payloads.
use nxformat_packer::read_profile::CapturedReadProfiles;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: profile_inspect TRACE.tsv")?;
    let profiles = CapturedReadProfiles::read(std::fs::File::open(path)?)?;
    for f in profiles.files {
        println!(
            "{:016x}\t{:?}\t{}\t{}\t{}\t{}\t{}\t{}",
            f.scope.application_id,
            f.scope.program_id,
            f.scope.numeric_version,
            f.path,
            f.size,
            f.profile.observations(),
            f.profile.largest_read(),
            f.profile.contiguous_reads()
        );
    }
    Ok(())
}
