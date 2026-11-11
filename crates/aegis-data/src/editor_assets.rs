//! Optional immutable local frontend bundle. No CDN or runtime package fetching.
use anyhow::{bail, Result};
use axum::body::Bytes;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path},
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    monaco_version: String,
    entry: String,
    files: BTreeMap<String, File>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    sha256: String,
    size_bytes: usize,
}
pub struct Asset {
    pub bytes: Bytes,
    pub mime: &'static str,
}
pub struct Assets {
    pub files: BTreeMap<String, Asset>,
    manifest_sha256: String,
    monaco_version: String,
}
fn read(root: &Path, relative: &str, limit: usize) -> Result<Vec<u8>> {
    let path = Path::new(relative);
    if relative.is_empty()
        || relative.len() > 512
        || path.is_absolute()
        || !path.components().all(|p| matches!(p, Component::Normal(_)))
    {
        bail!("invalid editor asset path")
    }
    let mut file = root.to_path_buf();
    for component in path.components() {
        file.push(component);
        if fs::symlink_metadata(&file)?.file_type().is_symlink() {
            bail!("symlink editor assets denied")
        }
    }
    if !file.is_file() || fs::metadata(&file)?.len() > limit as u64 {
        bail!("asset must be a bounded regular file")
    }
    let mut bytes = Vec::new();
    fs::File::open(file)?
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        bail!("asset grew beyond limit")
    }
    Ok(bytes)
}
impl Assets {
    pub fn load(root: &Path, expected: &str) -> Result<Self> {
        let root = root.canonicalize()?;
        let manifest_bytes = read(&root, "manifest.json", 2 * 1024 * 1024)?;
        let manifest_sha256 = crate::ingest::hash(&manifest_bytes);
        if manifest_sha256 != expected {
            bail!("editor bundle manifest hash mismatch")
        }
        let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;
        if manifest.schema_version != 1
            || manifest.entry != "editor.js"
            || !manifest.files.contains_key("editor.js")
            || !manifest.files.contains_key("editor.css")
            || !manifest.files.contains_key("editor.worker.js")
            || manifest.files.len() > 1024
        {
            bail!("invalid editor bundle contract")
        }
        let mut files = BTreeMap::new();
        let mut total = 0usize;
        for (path, entry) in manifest.files {
            if entry.size_bytes > 20 * 1024 * 1024 {
                bail!("editor asset exceeds 20 MiB")
            }
            total = total.saturating_add(entry.size_bytes);
            if total > 64 * 1024 * 1024 {
                bail!("editor bundle exceeds 64 MiB")
            }
            let bytes = read(&root, &path, entry.size_bytes)?;
            if bytes.len() != entry.size_bytes || crate::ingest::hash(&bytes) != entry.sha256 {
                bail!("editor asset identity mismatch: {path}")
            }
            let mime = match Path::new(&path).extension().and_then(|s| s.to_str()) {
                Some("js") => "text/javascript; charset=utf-8",
                Some("css") => "text/css; charset=utf-8",
                Some("ttf") => "font/ttf",
                Some("txt") => "text/plain; charset=utf-8",
                _ => bail!("unsupported editor asset type"),
            };
            files.insert(
                path,
                Asset {
                    bytes: Bytes::from(bytes),
                    mime,
                },
            );
        }
        Ok(Self {
            files,
            manifest_sha256,
            monaco_version: manifest.monaco_version,
        })
    }
    pub fn summary(&self) -> Value {
        json!({"enabled":true,"manifest_sha256":self.manifest_sha256,"monaco_version":self.monaco_version,"files":self.files.len(),"local_assets_only":true})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundle_anchor_members_and_symlinks_are_checked() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        let bytes = b"example";
        let mut files = serde_json::Map::new();
        for name in ["editor.js", "editor.css", "editor.worker.js"] {
            fs::write(root.join(name), bytes).unwrap();
            files.insert(
                name.into(),
                json!({"sha256":crate::ingest::hash(bytes),"size_bytes":bytes.len()}),
            );
        }
        let manifest=serde_json::to_vec(&json!({"schema_version":1,"monaco_version":"fixture","entry":"editor.js","files":files})).unwrap();
        fs::write(root.join("manifest.json"), &manifest).unwrap();
        let anchor = crate::ingest::hash(&manifest);
        assert!(Assets::load(root, &anchor).is_ok());
        assert!(Assets::load(root, "wrong").is_err());
        fs::write(root.join("editor.js"), b"tampered").unwrap();
        assert!(Assets::load(root, &anchor).is_err());
        #[cfg(unix)]
        {
            fs::remove_file(root.join("editor.js")).unwrap();
            std::os::unix::fs::symlink(root.join("editor.css"), root.join("editor.js")).unwrap();
            assert!(Assets::load(root, &anchor).is_err());
        }
    }
}
