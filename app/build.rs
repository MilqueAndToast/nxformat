fn main() {
    println!("cargo:rerun-if-changed=assets/nxconvert.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/nxconvert.ico");
        res.compile().expect("embed the application icon");
    }
}
