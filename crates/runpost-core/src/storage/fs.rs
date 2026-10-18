//! Utilidades de disco: escritura atómica y nombres de archivo.

use std::{
    fs,
    io::{self, Write},
    path::Path,
    thread,
    time::Duration,
};

use anyhow::{Context, Result};

/// Escribe en un temporal del mismo directorio, `fsync` y `rename` sobre el destino.
/// Un lector nunca ve un archivo a medias. En Windows el `rename` puede fallar
/// mientras otro proceso tiene abierto el destino; se reintenta brevemente.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("path without parent")?;
    let name = path
        .file_name()
        .context("path without file name")?
        .to_string_lossy();
    let tmp = dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
    let written = (|| -> io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(error) = written {
        let _ = fs::remove_file(&tmp);
        return Err(error).with_context(|| format!("cannot write {}", tmp.display()));
    }
    let mut attempt = 0;
    loop {
        match fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(error) if attempt < 20 && error.kind() == io::ErrorKind::PermissionDenied => {
                attempt += 1;
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => {
                let _ = fs::remove_file(&tmp);
                return Err(error).with_context(|| format!("cannot replace {}", path.display()));
            }
        }
    }
}

/// JSON con sangría de 2 espacios y salto final: diffs legibles en git.
pub fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    write_atomic(path, text.as_bytes())
}

/// `"Acme Gateway - Validación"` → `"acme-gateway-validacion"`.
pub fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    for ch in name.chars().flat_map(char::to_lowercase) {
        let ch = match ch {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            other => other,
        };
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("item");
    }
    slug
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(
            slugify("Acme Gateway - Validación de scopes"),
            "acme-gateway-validacion-de-scopes"
        );
        assert_eq!(slugify("Acme-Gateway-Scopes"), "acme-gateway-scopes");
        assert_eq!(slugify("  ¿Ñandú?  "), "nandu");
        assert_eq!(slugify("***"), "item");
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("a.json");
        write_atomic(&path, b"one")?;
        write_atomic(&path, b"two")?;
        assert_eq!(fs::read_to_string(&path)?, "two");
        let entries: Vec<_> = fs::read_dir(dir.path())?.collect();
        assert_eq!(entries.len(), 1);
        Ok(())
    }
}
