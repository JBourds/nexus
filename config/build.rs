fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let modules_dir = std::path::Path::new(&manifest_dir)
        .parent()
        .unwrap()
        .join("modules");
    println!("cargo:rustc-env=NEXUS_STDLIB_DIR={}", modules_dir.display());
}
