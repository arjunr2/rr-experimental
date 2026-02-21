use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let driver_manifest = manifest_dir
        .join("../crimp-glue/driver/Cargo.toml")
        .canonicalize()
        .expect("crimp-glue/driver/Cargo.toml should exist relative to decomposer");

    println!(
        "cargo:rustc-env=CRIMP_DRIVER_MANIFEST={}",
        driver_manifest.display()
    );
}
