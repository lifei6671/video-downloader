fn main() {
    #[cfg(feature = "desktop")]
    tauri_build::build();

    // Tauri emits rustc-link-arg-bins for this resource. Cargo examples need
    // the same manifest and icons to run the production dialog/tray entry point.
    #[cfg(feature = "desktop")]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let resource = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"))
            .join("resource.lib");
        println!("cargo:rustc-link-arg-examples={}", resource.display());
    }
}
