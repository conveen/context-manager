fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        // tauri-build embeds its Common-Controls-v6 application manifest as a
        // Windows resource, which embed-resource links into *bin* targets
        // only. The lib unit-test binary therefore starts without a manifest,
        // resolves Common Controls v5, and dies on launch with
        // STATUS_ENTRYPOINT_NOT_FOUND (tauri-apps/tauri#13419) — and
        // `rustc-link-arg-tests` can't help, since it only reaches
        // integration-test targets. Instead, suppress tauri-build's manifest
        // resource and embed the same manifest through the MSVC linker, which
        // applies to every executable this package links: the app binary and
        // the test binaries alike.
        let manifest =
            std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.to_str().unwrap());
        tauri_build::try_build(
            tauri_build::Attributes::new()
                .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()),
        )
        .expect("failed to run tauri-build");
    } else {
        tauri_build::build()
    }
}
