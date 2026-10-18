fn main() {
    println!("cargo:rerun-if-changed=../../ui/dist");
    if std::env::var_os("CARGO_FEATURE_EMBED_UI").is_some()
        && !std::path::Path::new("../../ui/dist/index.html").is_file()
    {
        panic!(
            "ui/dist/index.html no existe: ejecuta scripts/build.bat (o scripts/build.sh) primero"
        );
    }
}
