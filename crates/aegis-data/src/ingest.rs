use crate::wire::Sample;
use anyhow::{bail, Context, Result};
use prost::Message;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
};
use tokio::sync::mpsc;
const MAX_RECORD: usize = 4 * 1024 * 1024;
#[derive(Deserialize)]
struct Input {
    text: String,
    source: String,
    license: String,
    revision: String,
    group_id: String,
    #[serde(default = "code_kind")]
    content_kind: String,
    #[serde(default = "preserve_mode")]
    normalization: String,
}
fn code_kind() -> String {
    "code".into()
}
fn preserve_mode() -> String {
    "preserve".into()
}
#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Shard {
    pub sha256: String,
    pub size_bytes: u64,
    pub records: u64,
}
#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Summary {
    pub schema_version: u32,
    pub transformations: Vec<String>,
    pub shards: std::collections::BTreeMap<String, Shard>,
    #[serde(default)]
    pub normalization_counts: std::collections::BTreeMap<String, u64>,
    pub train: u64,
    pub validation: u64,
    pub test: u64,
    pub duplicates: u64,
    pub sha256: String,
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn split(group: &str) -> &'static str {
    let bytes = Sha256::digest(group.as_bytes());
    let bucket = u32::from_be_bytes(bytes[..4].try_into().unwrap()) % 100;
    if bucket < 5 {
        "test"
    } else if bucket < 10 {
        "validation"
    } else {
        "train"
    }
}
fn read_bounded<R: BufRead>(r: &mut R) -> Result<Option<Vec<u8>>> {
    let mut out = Vec::new();
    loop {
        let buf = r.fill_buf()?;
        if buf.is_empty() {
            return Ok(if out.is_empty() { None } else { Some(out) });
        }
        let n = buf
            .iter()
            .position(|&v| v == b'\n')
            .map_or(buf.len(), |i| i + 1);
        if out.len() + n > MAX_RECORD {
            bail!("record exceeds 4 MiB")
        }
        out.extend_from_slice(&buf[..n]);
        let ended = out.last() == Some(&b'\n');
        r.consume(n);
        if ended {
            return Ok(Some(out));
        }
    }
}
pub async fn prepare(input: PathBuf, out: PathBuf, capacity: usize) -> Result<Summary> {
    if capacity == 0 || capacity > 64 {
        bail!("queue capacity must be 1..64")
    }
    fs::create_dir(&out).context("output must be a fresh directory")?;
    let (tx, mut rx) = mpsc::channel::<Sample>(capacity);
    let producer = tokio::task::spawn_blocking(move || -> Result<()> {
        let mut f = BufReader::new(File::open(input)?);
        while let Some(raw) = read_bounded(&mut f)? {
            let r: Input = serde_json::from_slice(&raw)?;
            if [
                r.source.trim(),
                r.license.trim(),
                r.revision.trim(),
                r.group_id.trim(),
            ]
            .iter()
            .any(|s| s.is_empty())
            {
                bail!("source/license/revision/group_id must be nonempty")
            }
            if !["code", "prose"].contains(&r.content_kind.as_str())
                || !["preserve", "collapse_whitespace"].contains(&r.normalization.as_str())
                || r.content_kind == "code" && r.normalization != "preserve"
            {
                bail!("code must be preserved; prose normalization must be explicit")
            }
            if r.group_id.len() > 256
                || r.source.len() > 4096
                || r.revision.len() > 1024
                || r.license.len() > 1024
            {
                bail!("provenance metadata exceeds limits")
            }
            let original_sha256 = hash(r.text.as_bytes());
            let text = if r.normalization == "preserve" {
                r.text
            } else {
                r.text.split_whitespace().collect::<Vec<_>>().join(" ")
            };
            if text.is_empty() {
                continue;
            }
            let sample = Sample {
                sha256: hash(text.as_bytes()),
                source: r.source,
                license: r.license,
                group_id: r.group_id.clone(),
                split: split(&r.group_id).into(),
                text,
                revision: r.revision,
                content_kind: r.content_kind,
                normalization: r.normalization,
                original_sha256,
            };
            tx.blocking_send(sample)
                .map_err(|_| anyhow::anyhow!("writer stopped"))?;
        }
        Ok(())
    });
    // A single writer owns ordering and dedup. Bounded queue applies backpressure.
    let result = (|| -> Result<_> {
        let db = Connection::open(out.join("dedup.sqlite"))?;
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE seen(sha TEXT PRIMARY KEY, split TEXT NOT NULL);")?;
        let train = BufWriter::new(File::create(out.join("train.pb"))?);
        let validation = BufWriter::new(File::create(out.join("validation.pb"))?);
        let test = BufWriter::new(File::create(out.join("test.pb"))?);
        Ok((db, train, validation, test))
    })();
    let (db, mut train, mut validation, mut test) = match result {
        Ok(v) => v,
        Err(e) => {
            drop(rx);
            let _ = producer.await;
            return Err(e);
        }
    };
    let mut summary = Summary::default();
    let mut hasher = Sha256::new();
    let processing = async {
        while let Some(s) = rx.recv().await {
            *summary
                .normalization_counts
                .entry(format!("{}:{}", s.content_kind, s.normalization))
                .or_default() += 1;
            let previous: Option<String> = db
                .query_row("SELECT split FROM seen WHERE sha=?1", [&s.sha256], |r| {
                    r.get(0)
                })
                .optional()?;
            if let Some(previous) = previous {
                if previous != s.split {
                    bail!("duplicate content crosses splits; merge related group IDs and rerun")
                }
                summary.duplicates += 1;
                continue;
            }
            db.execute(
                "INSERT INTO seen VALUES (?1,?2)",
                params![s.sha256, s.split],
            )?;
            let writer = match s.split.as_str() {
                "train" => {
                    summary.train += 1;
                    &mut train
                }
                "validation" => {
                    summary.validation += 1;
                    &mut validation
                }
                _ => {
                    summary.test += 1;
                    &mut test
                }
            };
            let payload = s.encode_to_vec();
            let size = u32::try_from(payload.len())?;
            writer.write_all(&size.to_le_bytes())?;
            writer.write_all(&payload)?;
            hasher.update(size.to_le_bytes());
            hasher.update(payload);
        }
        anyhow::Ok(())
    }
    .await;
    drop(rx);
    let produced = producer.await?;
    processing?;
    produced?;
    train.flush()?;
    validation.flush()?;
    test.flush()?;
    train.get_ref().sync_all()?;
    validation.get_ref().sync_all()?;
    test.get_ref().sync_all()?;
    summary.sha256 = format!("{:x}", hasher.finalize());
    summary.schema_version = 2;
    summary.transformations = vec![
        "content_aware_preservation_v2".into(),
        "exact_sha256_dedup_v1".into(),
        "group_sha256_split_90_5_5_v1".into(),
    ];
    for (name, records) in [
        ("train", summary.train),
        ("validation", summary.validation),
        ("test", summary.test),
    ] {
        let path = out.join(format!("{name}.pb"));
        summary.shards.insert(
            format!("{name}.pb"),
            Shard {
                sha256: hash_file(&path)?,
                size_bytes: fs::metadata(path)?.len(),
                records,
            },
        );
    }
    fs::write(
        out.join("manifest.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    fs::write(out.join("COMPLETE"), b"v1\n")?;
    Ok(summary)
}
use rusqlite::OptionalExtension;
pub fn hash_file(path: &Path) -> Result<String> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut b = [0u8; 65536];
    loop {
        let n = f.read(&mut b)?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
pub fn validate_dataset(root: &Path) -> Result<Summary> {
    if fs::read(root.join("COMPLETE"))? != b"v1\n" {
        bail!("missing completion marker")
    }
    let summary: Summary = serde_json::from_slice(&fs::read(root.join("manifest.json"))?)?;
    if summary.schema_version != 2 || summary.shards.len() != 3 {
        bail!("unsupported dataset manifest")
    }
    for (name, records) in [
        ("train.pb", summary.train),
        ("validation.pb", summary.validation),
        ("test.pb", summary.test),
    ] {
        let s = summary.shards.get(name).context("missing shard entry")?;
        let p = root.join(name);
        if s.records != records
            || fs::metadata(&p)?.len() != s.size_bytes
            || hash_file(&p)? != s.sha256
            || inspect(&p)? != records
        {
            bail!("shard hash/size/record mismatch: {name}")
        }
    }
    Ok(summary)
}
pub fn inspect(path: &Path) -> Result<u64> {
    let mut f = BufReader::new(File::open(path)?);
    let mut count = 0;
    loop {
        let mut first = [0u8; 1];
        if f.read(&mut first)? == 0 {
            break;
        }
        let mut size = [0u8; 4];
        size[0] = first[0];
        f.read_exact(&mut size[1..])?;
        let n = u32::from_le_bytes(size) as usize;
        if n > MAX_RECORD + 1024 {
            bail!("oversize frame")
        }
        let mut payload = vec![0; n];
        f.read_exact(&mut payload)?;
        let s = Sample::decode(payload.as_slice())?;
        let expected = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| ["train", "validation", "test"].contains(s));
        if !["code", "prose"].contains(&s.content_kind.as_str())
            || !["preserve", "collapse_whitespace"].contains(&s.normalization.as_str())
            || s.content_kind == "code" && s.normalization != "preserve"
            || s.original_sha256.len() != 64
            || s.normalization == "preserve" && s.original_sha256 != s.sha256
        {
            bail!("invalid content-preservation metadata")
        }
        if s.text.is_empty()
            || s.source.is_empty()
            || s.license.is_empty()
            || s.revision.is_empty()
            || s.group_id.is_empty()
            || expected.is_some_and(|v| s.split != v)
            || s.sha256 != hash(s.text.as_bytes())
            || s.split != split(&s.group_id)
        {
            bail!("sample integrity mismatch")
        }
        count += 1;
    }
    Ok(count)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn roundtrip_and_dedup() {
        let t = tempfile::tempdir().unwrap();
        let input = t.path().join("in");
        let row = r#"{"text":"hello world","source":"fixture","license":"CC0","revision":"v1","group_id":"doc1"}"#;
        fs::write(&input, format!("{row}\n{row}\n")).unwrap();
        let out = t.path().join("out");
        let s = prepare(input, out.clone(), 2).await.unwrap();
        assert_eq!(s.duplicates, 1);
        let n = inspect(&out.join("train.pb")).unwrap()
            + inspect(&out.join("validation.pb")).unwrap()
            + inspect(&out.join("test.pb")).unwrap();
        assert_eq!(n, 1);
        assert!(out.join("COMPLETE").exists());
        validate_dataset(&out).unwrap();
        fs::write(out.join("train.pb"), b"corrupt").unwrap();
        assert!(validate_dataset(&out).is_err());
    }
    #[tokio::test]
    async fn code_bytes_preserved() {
        let d = tempfile::tempdir().unwrap();
        let input = d.path().join("in");
        let code = "def f():\r\n    return \"a  b\"\r\n";
        let row = serde_json::json!({"text":code,"source":"code.py","license":"local","revision":"v1","group_id":"doc"});
        fs::write(&input, format!("{row}\n")).unwrap();
        let out = d.path().join("out");
        prepare(input, out.clone(), 1).await.unwrap();
        let data = fs::read(out.join(format!("{}.pb", split("doc")))).unwrap();
        let n = u32::from_le_bytes(data[..4].try_into().unwrap()) as usize;
        let s = Sample::decode(&data[4..4 + n]).unwrap();
        assert_eq!(s.text.as_bytes(), code.as_bytes());
        assert_eq!(s.original_sha256, s.sha256);
    }
    #[tokio::test]
    async fn code_normalization_rejected() {
        let d = tempfile::tempdir().unwrap();
        let input = d.path().join("in");
        fs::write(&input,serde_json::json!({"text":"a  b","source":"x","license":"x","revision":"v1","group_id":"doc","normalization":"collapse_whitespace"}).to_string()).unwrap();
        assert!(prepare(input, d.path().join("out"), 1).await.is_err());
    }
    #[test]
    fn truncated_rejected() {
        let t = tempfile::NamedTempFile::new().unwrap();
        fs::write(t.path(), [8, 0]).unwrap();
        assert!(inspect(t.path()).is_err());
    }
    #[test]
    fn oversized_line_rejected() {
        let v = vec![b'x'; MAX_RECORD + 1];
        assert!(read_bounded(&mut BufReader::new(v.as_slice())).is_err());
    }
    #[tokio::test]
    async fn failure_has_no_complete() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("bad");
        fs::write(&src, b"{}\n").unwrap();
        let out = t.path().join("out");
        assert!(prepare(src, out.clone(), 1).await.is_err());
        assert!(!out.join("COMPLETE").exists());
    }
}
