fn main() {
    println!("cargo:rerun-if-changed=resources/windows.manifest");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
        && std::env::var_os("CARGO_FEATURE_GUI").is_some()
    {
        let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .join("resources/windows.manifest");
        println!("cargo:rustc-link-arg-bin=timelapse4sai2-gui=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=timelapse4sai2-gui=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
}
