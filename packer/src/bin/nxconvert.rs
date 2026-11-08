//! Command-line front of [`nxformat_packer::cli`].
fn main() {
    if let Err(e) = nxformat_packer::cli::run(std::env::args().skip(1).collect(), None, None) {
        eprintln!("ERROR: {e}");
        std::process::exit(1);
    }
}
