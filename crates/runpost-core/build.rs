use std::{env, fs, path::PathBuf};

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let manifest = root.join("Cargo.toml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let config: toml::Value = toml::from_str(&fs::read_to_string(manifest).unwrap()).unwrap();
    let name = config["workspace"]["metadata"]["app"]["name"]
        .as_str()
        .unwrap();
    let slug = name.to_ascii_lowercase();
    // SPEC-DEVIATION: S0-D1 — nombre del ejecutable CLI/MCP elegido por Paul.
    let cli_bin = config["workspace"]["metadata"]["app"]["cli-bin"]
        .as_str()
        .unwrap();
    let constants = format!(
        "pub const APP_NAME: &str = {name:?};\npub const APP_SLUG: &str = {slug:?};\npub const WORKSPACE_ENV: &str = {:?};\npub const TOKEN_HEADER: &str = {:?};\npub const CLI_BIN: &str = {cli_bin:?};\n",
        format!("{}_WORKSPACE", slug.to_ascii_uppercase()),
        format!("X-{name}-Token"),
    );
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("app.rs"),
        constants,
    )
    .unwrap();
}
