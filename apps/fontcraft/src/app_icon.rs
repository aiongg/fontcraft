//! The app icon, everywhere Windows, macOS and Linux show one.
//!
//! - **Executable:** `build.rs` embeds `assets/app-icon/fontcraft.ico` (16–256 px) as icon resource 1, which
//!   Explorer, Alt+Tab and an unpinned taskbar button fall back to.
//! - **Window:** [`viewport_icon`] sets the title-bar and taskbar icon at runtime (`WM_SETICON` on Windows,
//!   `_NET_WM_ICON` on X11, the Dock when running unbundled on macOS). Wayland has no window icon: the dock shows the
//!   icon of the `.desktop` file whose name matches the window's app id.

use eframe::egui;

/// Window, taskbar and (when running unbundled) Dock icon. macOS gets the render on Apple's icon grid, with its
/// transparent margin; elsewhere the 256 px hicolor render of the full tile.
pub fn window_icon() -> egui::IconData {
    match eframe::icon_data::from_png_bytes(PNG) {
        Ok(icon) => icon,
        Err(e) => {
            eprintln!("fontcraft: app icon: {e}");
            egui::IconData::default()
        }
    }
}

/// Whether the window gets [`window_icon`] at runtime: everywhere except inside a macOS app bundle. There the Dock
/// shows the bundle's `.icns` (`CFBundleIconFile`), which macOS renders in its own style (its tinted and dark
/// variants); a runtime PNG would replace it with a flat picture. `exe` is the running executable.
pub fn sets_runtime_icon(exe: Option<&std::path::Path>) -> bool {
    !(cfg!(target_os = "macos") && exe.is_some_and(in_app_bundle))
}

/// The icon for the root viewport: [`window_icon`] when [`sets_runtime_icon`] is true; otherwise an empty icon, which
/// keeps eframe from putting its own icon on the Dock once the window opens.
pub fn viewport_icon(exe: Option<&std::path::Path>) -> egui::IconData {
    if sets_runtime_icon(exe) { window_icon() } else { egui::IconData::default() }
}

/// Whether `exe` is an app bundle's executable: `…/Name.app/Contents/MacOS/binary`.
fn in_app_bundle(exe: &std::path::Path) -> bool {
    let name = |p: Option<&std::path::Path>| p.and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
    let macos = exe.parent();
    let contents = macos.and_then(std::path::Path::parent);
    let bundle = contents.and_then(std::path::Path::parent);
    name(macos).as_deref() == Some("MacOS")
        && name(contents).as_deref() == Some("Contents")
        && name(bundle).is_some_and(|n| n.len() > 4 && n.to_ascii_lowercase().ends_with(".app"))
}

#[cfg(target_os = "macos")]
const PNG: &[u8] = include_bytes!("../../../assets/app-icon/fontcraft-macos-512.png");
#[cfg(not(target_os = "macos"))]
const PNG: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.fontcraft.png");

#[cfg(test)]
mod tests {
    use super::*;

    const ICO: &[u8] = include_bytes!("../../../assets/app-icon/fontcraft.ico");

    fn u16_at(b: &[u8], i: usize) -> usize {
        u16::from_le_bytes([b[i], b[i + 1]]) as usize
    }
    fn u32_at(b: &[u8], i: usize) -> usize {
        u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize
    }

    #[test]
    fn only_a_macos_app_bundle_skips_the_runtime_icon() {
        use std::path::Path;
        assert!(in_app_bundle(Path::new("/Applications/FontCraft.app/Contents/MacOS/fontcraft")));
        assert!(in_app_bundle(Path::new("/Users/a/Downloads/FontCraft Beta.APP/Contents/MacOS/fontcraft")));
        assert!(!in_app_bundle(Path::new("/Users/a/src/fontcraft/target/release/fontcraft")));
        assert!(!in_app_bundle(Path::new("/opt/Contents/MacOS/fontcraft")), "not inside a .app");
        assert!(!in_app_bundle(Path::new("/.app/Contents/MacOS/fontcraft")), "a bare `.app` is no bundle name");
        assert!(!in_app_bundle(Path::new("fontcraft")));
        assert!(sets_runtime_icon(None));
        assert!(sets_runtime_icon(Some(Path::new("/usr/bin/fontcraft"))));
        let bundled = Some(Path::new("/Applications/FontCraft.app/Contents/MacOS/fontcraft"));
        assert_eq!(sets_runtime_icon(bundled), !cfg!(target_os = "macos"));
        if !sets_runtime_icon(bundled) {
            assert_eq!(viewport_icon(bundled), egui::IconData::default());
        }
    }

    #[test]
    fn window_icon_decodes() {
        let icon = window_icon();
        assert!(icon.width >= 256 && icon.width == icon.height, "{}x{}", icon.width, icon.height);
        assert_eq!(icon.rgba.len(), icon.width as usize * icon.height as usize * 4);
        assert!(icon.rgba.chunks(4).any(|p| p[3] > 0), "the icon isn't blank");
    }

    #[test]
    fn windows_and_linux_icons_keep_the_round_corners() {
        // rx=112 of 512 leaves (10, 10) of a 256 px render transparent.
        let png = include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.fontcraft.png");
        let icon = eframe::icon_data::from_png_bytes(png).unwrap();
        assert_eq!(icon.width, 256);
        assert_eq!(icon.rgba[(10 * 256 + 10) * 4 + 3], 0, "corner is transparent");
        assert_eq!(icon.rgba[(128 * 256 + 2) * 4 + 3], 255, "the tile is full bleed");
    }

    /// The `.ico` build.rs embeds: every size Windows asks for, each a valid PNG or BMP image.
    #[test]
    fn exe_icon_has_every_shell_size() {
        assert_eq!((u16_at(ICO, 0), u16_at(ICO, 2)), (0, 1), "ICONDIR header, type 1 (icon)");
        let n = u16_at(ICO, 4);
        let mut sizes = Vec::new();
        for i in 0..n {
            let e = 6 + i * 16;
            let w = if ICO[e] == 0 { 256 } else { ICO[e] as usize };
            let h = if ICO[e + 1] == 0 { 256 } else { ICO[e + 1] as usize };
            assert_eq!(w, h);
            let (len, off) = (u32_at(ICO, e + 8), u32_at(ICO, e + 12));
            let data = ICO.get(off..off + len).expect("image data inside the file");
            let png = data.starts_with(b"\x89PNG\r\n\x1a\n");
            let bmp = data.len() >= 40 && u32_at(data, 0) == 40;
            assert!(png || bmp, "{w}px entry is neither PNG nor BMP");
            if png {
                let pw = u32::from_be_bytes([data[16], data[17], data[18], data[19]]) as usize;
                assert_eq!(pw, w, "{w}px entry holds a {pw}px PNG");
            }
            sizes.push(w);
        }
        for want in [16, 24, 32, 48, 64, 256] {
            assert!(sizes.contains(&want), "fontcraft.ico lacks {want}px (has {sizes:?})");
        }
    }
}
