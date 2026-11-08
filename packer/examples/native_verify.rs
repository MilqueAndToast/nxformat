//! Standalone validation entry point for long-running private comparisons.
#[path = "support/native_verify.rs"]
mod native_verify;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let paths: Vec<_> = args[2..].iter().map(std::path::PathBuf::from).collect();
    native_verify::verify(
        std::path::Path::new(&args[1]),
        &paths,
        &nxformat_packer::WriterOptions::default(),
    )
}
