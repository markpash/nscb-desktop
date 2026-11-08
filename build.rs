fn main() {
    println!("cargo:rerun-if-changed=assets/app-icons/icon.ico");

    // Embed the application icon into the Windows executable.
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/app-icons/icon.ico");
        res.compile().expect("failed to embed Windows icon resource");
    }
}
