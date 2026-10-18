fn main() {
    // productName se obtiene del único nombre de producto en Cargo.toml.
    let path = std::path::Path::new("../../../Cargo.toml");
    println!("cargo:rerun-if-changed={}", path.display());
    let manifest: toml::Value = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let app = &manifest["workspace"]["metadata"]["app"];
    println!(
        "cargo:rustc-env=RUNPOST_PRODUCT_NAME={}",
        app["name"].as_str().unwrap()
    );
    tauri_build::build();
}
