fn main() {
    // Embeds the app icon into the Windows executable.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("packaging/kyu.ico");
        res.compile().expect("failed to embed the Windows icon");
    }
}
