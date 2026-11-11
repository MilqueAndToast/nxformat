// Headless build for testing the same commands without GUI dependencies.
fn main() {
    if let Err(error) = nxformat_packer::cli::run(std::env::args().skip(1).collect(), None, None) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
