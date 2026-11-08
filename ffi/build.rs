// The shared library is found by name at run time, wherever it is installed.
fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match os.as_str() {
        "macos" | "ios" => {
            println!("cargo:rustc-cdylib-link-arg=-Wl,-install_name,@rpath/libnxformat.dylib")
        }
        "linux" | "android" => println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libnxformat.so"),
        _ => {}
    }
}
