//! Build script: embeds the application icon (and version details from `Cargo.toml`)
//! in the Windows executable, so Explorer and the taskbar show the Mind Mapper icon.

fn main() {
    // `cargo:` lines printed by a build script are instructions to Cargo, not log output.
    // Only re-run this script when the icon changes, not on every source edit.
    println!("cargo:rerun-if-changed=assets/icon.ico");

    // The build-dependency only exists on Windows hosts; the target check skips it when
    // building for another platform.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .compile()
            .expect("embedding the icon needs the Windows SDK resource compiler (rc.exe)");
    }
}
