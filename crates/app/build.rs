//! On Windows, embeds the app icon and version info into opentern.exe, so
//! Explorer, the taskbar, Start search and Settings → Apps show the Opentern
//! icon. Adapted from Magpie's.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/opentern.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/opentern.ico")
            .set("ProductName", "Opentern")
            .set("FileDescription", "Opentern")
            .set("CompanyName", "Ahaan Shah")
            .set("LegalCopyright", "Copyright (c) 2026 Ahaan Shah. MIT License.")
            .set("OriginalFilename", "opentern.exe")
            .set("InternalName", "opentern");
        res.compile().expect("embed Windows resources");
    }
}
