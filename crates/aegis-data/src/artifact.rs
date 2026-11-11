//! Resumable chunk transport between filesystem stores, separate from worker RPC.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub size: u64,
    pub chunk_bytes: u64,
    pub sha256: String,
    pub chunks: Vec<String>,
}
fn digest(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn regular(path: &Path) -> Result<()> {
    if !fs::symlink_metadata(path)?.is_file() {
        bail!("regular file required")
    }
    Ok(())
}
fn store(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    if !fs::symlink_metadata(path)?.is_dir() {
        bail!("store must be a directory, not a symlink")
    }
    Ok(())
}
fn load(path: &Path) -> Result<Manifest> {
    regular(path)?;
    if fs::metadata(path)?.len() > 80 * 1024 * 1024 {
        bail!("manifest too large")
    }
    let m: Manifest = serde_json::from_slice(&fs::read(path)?)?;
    let count = if m.size == 0 {
        0
    } else {
        (m.size - 1) / m.chunk_bytes.max(1) + 1
    };
    if m.schema_version != 1
        || !(1024 * 1024..=64 * 1024 * 1024).contains(&m.chunk_bytes)
        || !digest(&m.sha256)
        || m.chunks.len() > 1_000_000
        || count != m.chunks.len() as u64
        || m.chunks.iter().any(|s| !digest(s))
    {
        bail!("invalid artifact manifest")
    }
    Ok(m)
}
fn checked(path: &Path, hash: &str, size: u64) -> Result<()> {
    regular(path)?;
    if fs::metadata(path)?.len() != size || crate::ingest::hash_file(path)? != hash {
        bail!("artifact chunk integrity mismatch")
    }
    Ok(())
}
fn put(root: &Path, hash: &str, bytes: &[u8]) -> Result<bool> {
    let path = root.join(hash);
    if path.try_exists()? {
        checked(&path, hash, bytes.len() as u64)?;
        return Ok(false);
    }
    // create_new prevents replacement. Interrupted chunks are rejected on retry,
    // so write to a separate temporary inode before publishing by hard link.
    let tmp = root.join(format!(
        ".upload-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let result = (|| -> Result<bool> {
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        match fs::hard_link(&tmp, &path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                checked(&path, hash, bytes.len() as u64)?;
                Ok(false)
            }
            Err(e) => Err(e.into()),
        }
    })();
    let _ = fs::remove_file(tmp);
    result
}
pub fn pack(source: &Path, root: &Path, manifest: &Path, chunk_mib: u64) -> Result<Value> {
    if !(1..=64).contains(&chunk_mib) {
        bail!("chunk size must be 1..64 MiB")
    }
    regular(source)?;
    store(root)?;
    let initial = fs::metadata(source)?;
    let mut f = File::open(source)?;
    let chunk_bytes = chunk_mib * 1024 * 1024;
    if initial.len().div_ceil(chunk_bytes) > 1_000_000 {
        bail!("too many chunks")
    }
    let mut whole = Sha256::new();
    let mut chunks = Vec::new();
    let mut size = 0u64;
    let mut written = 0;
    loop {
        let mut bytes = Vec::with_capacity(chunk_bytes as usize);
        Read::by_ref(&mut f)
            .take(chunk_bytes)
            .read_to_end(&mut bytes)?;
        if bytes.is_empty() {
            break;
        }
        whole.update(&bytes);
        size += bytes.len() as u64;
        let hash = format!("{:x}", Sha256::digest(&bytes));
        if put(root, &hash, &bytes)? {
            written += 1;
        }
        chunks.push(hash);
        if chunks.len() > 1_000_000 {
            bail!("source exceeded chunk budget")
        }
    }
    let final_meta = fs::metadata(source)?;
    if initial.len() != size || initial.modified()? != final_meta.modified()? {
        bail!("source changed during packing")
    }
    let m = Manifest {
        schema_version: 1,
        size,
        chunk_bytes,
        sha256: format!("{:x}", whole.finalize()),
        chunks,
    };
    let encoded = serde_json::to_vec_pretty(&m)?;
    if manifest.try_exists()? {
        regular(manifest)?;
        if fs::read(manifest)? != encoded {
            bail!("manifest already exists with different identity")
        }
    } else {
        let tmp = manifest.with_file_name(format!(
            ".manifest-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let publication = (|| -> Result<()> {
            let mut out = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
            out.write_all(&encoded)?;
            out.sync_all()?;
            fs::hard_link(&tmp, manifest)?;
            #[cfg(unix)]
            File::open(
                manifest
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new(".")),
            )?
            .sync_all()?;
            Ok(())
        })();
        let _ = fs::remove_file(tmp);
        publication?;
    }
    Ok(
        json!({"manifest_sha256":crate::ingest::hash_file(manifest)?,"artifact_sha256":m.sha256,"size":size,"chunks":m.chunks.len(),"new_chunks":written,"scope":"filesystem chunk store; not worker RPC or checkpoint semantics"}),
    )
}
pub fn transfer(manifest: &Path, source: &Path, destination: &Path, anchor: &str) -> Result<Value> {
    if crate::ingest::hash_file(manifest)? != anchor {
        bail!("manifest anchor mismatch")
    }
    let m = load(manifest)?;
    store(destination)?;
    let mut transferred = 0;
    let mut reused = 0;
    for (i, hash) in m.chunks.iter().enumerate() {
        let size = (m.size - i as u64 * m.chunk_bytes).min(m.chunk_bytes);
        let target = destination.join(hash);
        if target.try_exists()? {
            checked(&target, hash, size)?;
            reused += 1;
            continue;
        }
        let path = source.join(hash);
        checked(&path, hash, size)?;
        let bytes = fs::read(path)?;
        if crate::ingest::hash(&bytes) != *hash {
            bail!("source changed during transfer")
        }
        if put(destination, hash, &bytes)? {
            transferred += 1;
        } else {
            reused += 1;
        }
    }
    Ok(
        json!({"transferred_chunks":transferred,"reused_chunks":reused,"size":m.size,"manifest_sha256":anchor}),
    )
}
pub fn materialize(manifest: &Path, root: &Path, out: &Path, anchor: &str) -> Result<Value> {
    if crate::ingest::hash_file(manifest)? != anchor {
        bail!("manifest anchor mismatch")
    }
    let m = load(manifest)?;
    if out.try_exists()? {
        checked(out, &m.sha256, m.size)?;
        return Ok(json!({"verified":true,"reused":true,"sha256":m.sha256}));
    }
    let partial = out.with_file_name(format!(
        "{}.partial",
        out.file_name()
            .ok_or_else(|| anyhow::anyhow!("output filename required"))?
            .to_string_lossy()
    ));
    if fs::symlink_metadata(&partial).is_ok() {
        regular(&partial)?;
    }
    let mut options = OpenOptions::new();
    options.read(true).append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut f = options.open(&partial)?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            bail!("another materializer owns this partial artifact")
        }
    }
    let length = f.metadata()?.len();
    let existing = if length == m.size {
        length
    } else {
        length / m.chunk_bytes * m.chunk_bytes
    };
    if length > m.size {
        bail!("partial artifact has invalid length; preserve or remove it explicitly")
    }
    let mut verified = 0u64;
    for (i, hash) in m.chunks.iter().enumerate() {
        let size = (m.size - i as u64 * m.chunk_bytes).min(m.chunk_bytes);
        if verified < existing {
            let mut bytes = vec![0; size as usize];
            f.read_exact(&mut bytes)?;
            if crate::ingest::hash(&bytes) != *hash {
                bail!("partial artifact does not match manifest")
            }
        } else {
            if f.metadata()?.len() != verified {
                f.set_len(verified)?;
            }
            let path = root.join(hash);
            checked(&path, hash, size)?;
            let bytes = fs::read(path)?;
            if crate::ingest::hash(&bytes) != *hash {
                bail!("chunk changed during materialization")
            }
            f.write_all(&bytes)?;
        }
        verified += size;
    }
    f.sync_all()?;
    drop(f);
    checked(&partial, &m.sha256, m.size)?;
    fs::hard_link(&partial, out)?;
    fs::remove_file(&partial)?;
    Ok(json!({"verified":true,"sha256":m.sha256,"size":m.size,"resumed_bytes":existing}))
}
