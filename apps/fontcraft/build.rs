//! Windows only: embed the app icon and version info (VERSIONINFO) into `fontcraft.exe`.
//!
//! On every other target this does nothing. A missing resource compiler is a warning, so a cross-compile from macOS
//! or Linux still links.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/fontcraft.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/fontcraft.ico")
        .set("ProductName", "FontCraft")
        .set("FileDescription", "FontCraft font editor")
        .set("LegalCopyright", "Copyright (c) the FontCraft authors. MIT OR Apache-2.0.")
        .set("OriginalFilename", "fontcraft.exe")
        .set("InternalName", "fontcraft");
    if let Err(e) = res.compile() {
        println!("cargo:warning=fontcraft.exe built without icon/version resources: {e}");
    }
}
