use std::path::PathBuf;

fn main() {
    let mono_manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let driver_mono_manifest = mono_manifest_dir
        .join("../crimp-drivers/mono/Cargo.toml")
        .canonicalize()
        .expect("crimp-drivers/mono/Cargo.toml should exist relative to decomposer");

    println!(
        "cargo:rustc-env=CRIMP_DRIVER_MONO_MANIFEST={}",
        driver_mono_manifest.display()
    );
}
