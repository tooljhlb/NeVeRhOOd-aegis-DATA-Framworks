//! Bounded local editor operations with preimage checks and recoverable snapshots.
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::Duration,
};
const LIMIT: u64 = 128 * 1024;
fn allowed(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path.to_string_lossy().len() <= 512
        && path.components().all(|c| match c {
            Component::Normal(name) => {
                let s = name.to_string_lossy();
                ![
                    ".git",
                    ".aegis",
                    "target",
                    "node_modules",
                    ".venv",
                    "venv",
                    "build",
                    "dist",
                    "__pycache__",
                ]
                .contains(&s.as_ref())
                    && !s.starts_with(".aegis-build-")
                    && !s.starts_with(".env")
                    && !["id_rsa", "id_ed25519"].contains(&s.as_ref())
                    && !["pem", "key", "p12"]
                        .iter()
                        .any(|ext| Path::new(name).extension().is_some_and(|e| e == *ext))
            }
            _ => false,
        })
}
fn resolve(root: &Path, relative: &str) -> Result<PathBuf> {
    let relative = Path::new(relative);
    if !allowed(relative) {
        bail!("editor path denied")
    }
    let root = root.canonicalize()?;
    let mut path = root.clone();
    for component in relative.components() {
        path.push(component);
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            bail!("editor refuses symlinks")
        }
    }
    if !path.canonicalize()?.starts_with(&root) {
        bail!("editor path outside workspace")
    }
    Ok(path)
}
fn bytes(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > LIMIT {
        bail!("editor requires regular UTF-8 files up to 128 KiB")
    }
    let mut content = Vec::new();
    File::open(path)?
        .take(LIMIT + 1)
        .read_to_end(&mut content)?;
    if content.len() as u64 > LIMIT {
        bail!("file grew beyond editor limit")
    }
    std::str::from_utf8(&content)?;
    if content.contains(&0) {
        bail!("binary files are not supported by the text editor")
    }
    Ok(content)
}
pub fn read(root: &Path, path: &str) -> Result<Value> {
    let content = bytes(&resolve(root, path)?)?;
    Ok(
        json!({"path":path,"sha256":crate::ingest::hash(&content),"text":std::str::from_utf8(&content)?,"bytes":content.len()}),
    )
}
pub fn tree(root: &Path) -> Result<Value> {
    let root = root.canonicalize()?;
    let mut files = Vec::new();
    let mut visited = 0;
    for entry in ignore::WalkBuilder::new(&root)
        .max_depth(Some(8))
        .hidden(false)
        .follow_links(false)
        .filter_entry({
            let root = root.clone();
            move |e| e.path() == root || e.path().strip_prefix(&root).is_ok_and(allowed)
        })
        .build()
    {
        let entry = entry?;
        visited += 1;
        if visited > 20000 {
            bail!("editor directory budget exceeded")
        }
        if entry.file_type().is_some_and(|t| t.is_file()) {
            if files.len() >= 5000 {
                bail!("editor file list limit exceeded; choose a smaller root")
            }
            let path = entry
                .path()
                .strip_prefix(&root)?
                .to_string_lossy()
                .into_owned();
            files.push(json!({"path":path,"bytes":entry.metadata()?.len()}));
        }
    }
    files.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    Ok(
        json!({"files":files,"scope":"bounded source file tree; generated, secret-like and symlink paths excluded"}),
    )
}
pub fn search(root: &Path, query: &str) -> Result<Value> {
    if query.is_empty() || query.len() > 256 {
        bail!("search requires 1..256 bytes")
    }
    let inventory = tree(root)?;
    let mut hits = Vec::new();
    let mut inspected = 0u64;
    for file in inventory["files"].as_array().unwrap() {
        let path = file["path"].as_str().unwrap();
        if file["bytes"].as_u64().unwrap_or(LIMIT + 1) > LIMIT {
            continue;
        }
        let Ok(content) = bytes(&resolve(root, path)?) else {
            continue;
        };
        inspected += content.len() as u64;
        if inspected > 16 * 1024 * 1024 {
            bail!("search byte budget exceeded; choose a smaller root")
        }
        for (n, line) in std::str::from_utf8(&content)?.lines().enumerate() {
            if line.contains(query) {
                hits.push(json!({"path":path,"line":n+1,"preview":line.chars().take(240).collect::<String>()}));
                if hits.len() == 200 {
                    return Ok(json!({"matches":hits,"truncated":true}));
                }
            }
        }
    }
    Ok(json!({"matches":hits,"truncated":false}))
}
fn private_dir(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    if !fs::symlink_metadata(path)?.is_dir() {
        bail!("editor metadata directory invalid")
    }
    Ok(())
}
pub fn save(root: &Path, path: &str, expected: &str, text: &str) -> Result<Value> {
    if text.len() as u64 > LIMIT || text.as_bytes().contains(&0) {
        bail!("replacement must be text without NUL bytes, up to 128 KiB")
    }
    let root = root.canonicalize()?;
    let target = resolve(&root, path)?;
    let metadata = root.join(".aegis");
    private_dir(&metadata)?;
    let history = metadata.join("editor");
    private_dir(&history)?;
    let lock_path = history.join("lock");
    if fs::symlink_metadata(&lock_path).is_ok_and(|m| !m.is_file()) {
        bail!("invalid editor lock")
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW).mode(0o600);
    }
    let lock = options.open(lock_path)?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            bail!("another editor operation owns this workspace")
        }
    }
    let original = bytes(&target)?;
    let before = crate::ingest::hash(&original);
    if before != expected {
        bail!("edit conflict: source differs from loaded revision")
    }
    let after = crate::ingest::hash(text.as_bytes());
    let backup = history.join(&before);
    if backup.try_exists()? {
        if crate::ingest::hash(&bytes(&backup)?) != before {
            bail!("editor backup integrity mismatch")
        }
    } else {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)?;
        f.write_all(&original)?;
        f.sync_all()?;
    }
    let id = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let receipt = history.join(format!("{id}.json"));
    let pending = json!({"schema_version":1,"path":path,"before_sha256":before,"after_sha256":after,"status":"pending","backup":before});
    let mut journal = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&receipt)?;
    journal.write_all(&serde_json::to_vec_pretty(&pending)?)?;
    journal.sync_all()?;
    let temporary = target.with_file_name(format!(".aegis-edit-{id}.tmp"));
    let operation = (|| -> Result<Value> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        f.write_all(text.as_bytes())?;
        f.set_permissions(fs::metadata(&target)?.permissions())?;
        f.sync_all()?;
        drop(f);
        if crate::ingest::hash(&bytes(&resolve(&root, path)?)?) != before {
            bail!("edit conflict during save")
        }
        fs::rename(&temporary, &target)?;
        #[cfg(unix)]
        File::open(target.parent().unwrap())?.sync_all()?;
        let result = json!({"schema_version":1,"path":path,"before_sha256":before,"sha256":after,"status":"saved","snapshot":before,"receipt":id});
        // A pending journal remains recoverable if the completion record cannot be written.
        fs::write(
            history.join(format!("{id}.complete.json")),
            serde_json::to_vec_pretty(&result)?,
        )?;
        Ok(result)
    })();
    if operation.is_err() {
        let _ = fs::remove_file(temporary);
    }
    operation
}
pub fn rollback(root: &Path, path: &str, expected: &str, snapshot: &str) -> Result<Value> {
    if snapshot.len() != 64 || !snapshot.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("invalid snapshot")
    }
    let root = root.canonicalize()?;
    let relative = format!(".aegis/editor/{snapshot}");
    let mut backup = root.clone();
    for c in Path::new(&relative).components() {
        backup.push(c);
        if fs::symlink_metadata(&backup)?.file_type().is_symlink() {
            bail!("snapshot symlink denied")
        }
    }
    let content = bytes(&backup)?;
    if crate::ingest::hash(&content) != snapshot {
        bail!("snapshot integrity mismatch")
    }
    save(&root, path, expected, std::str::from_utf8(&content)?)
}
pub fn git(root: &Path, mode: &str) -> Result<Value> {
    let args = match mode {
        "status" => vec!["status", "--porcelain=v1", "-uno"],
        "diff" => vec!["diff", "--no-ext-diff", "--no-textconv", "--"],
        _ => bail!("unknown Git view"),
    };
    let mut command = vec![
        "--no-pager",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "core.hooksPath=/dev/null",
    ];
    command.extend(args);
    let check = crate::code::Check {
        id: "workspace-git".into(),
        kind: "inspect".into(),
        program: "git".into(),
        args: command.into_iter().map(String::from).collect(),
        project_marker: None,
    };
    Ok(serde_json::to_value(crate::code::run(
        &root.canonicalize()?,
        &check,
        "git",
        mode,
        Duration::from_secs(10),
    )?)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_save_and_rollback_preserve_expected_preimages() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("code.rs"), "before").unwrap();
        let loaded = read(root.path(), "code.rs").unwrap();
        assert!(save(
            root.path(),
            "code.rs",
            loaded["sha256"].as_str().unwrap(),
            "bad\0text"
        )
        .is_err());
        let saved = save(
            root.path(),
            "code.rs",
            loaded["sha256"].as_str().unwrap(),
            "after",
        )
        .unwrap();
        assert!(save(
            root.path(),
            "code.rs",
            loaded["sha256"].as_str().unwrap(),
            "stale"
        )
        .is_err());
        assert_eq!(
            fs::read_to_string(root.path().join("code.rs")).unwrap(),
            "after"
        );
        rollback(
            root.path(),
            "code.rs",
            saved["sha256"].as_str().unwrap(),
            saved["snapshot"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(root.path().join("code.rs")).unwrap(),
            "before"
        );
    }
    #[test]
    fn workspace_boundaries_and_limits_are_enforced() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(".env"), "secret").unwrap();
        fs::write(root.path().join("big.txt"), vec![b'x'; LIMIT as usize + 1]).unwrap();
        assert!(read(root.path(), ".env").is_err());
        assert!(read(root.path(), "../outside").is_err());
        assert!(read(root.path(), "big.txt").is_err());
        fs::write(root.path().join("binary.dat"), b"ASCII\0binary").unwrap();
        assert!(read(root.path(), "binary.dat").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.path().join(".env"), root.path().join("link.txt"))
                .unwrap();
            assert!(read(root.path(), "link.txt").is_err());
        }
    }
}
