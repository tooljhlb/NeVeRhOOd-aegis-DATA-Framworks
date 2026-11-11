//! Optional, model-neutral retrieval and reviewed chat export. Never invokes a model.
use crate::ingest::hash;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_CHUNKS: usize = 4096;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub source: String,
    pub revision: String,
    pub license: String,
    pub group_id: String,
    pub text: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chunk {
    pub document: Document,
    pub start_byte: usize,
    pub end_byte: usize,
    pub document_sha256: String,
    pub chunk_sha256: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
    pub schema_version: u32,
    pub chunks: Vec<Chunk>,
    pub chunks_sha256: String,
}

fn nonempty(value: &str) -> bool {
    !value.trim().is_empty() && !value.contains('\0') && value.len() <= 4096
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        bail!("input exceeds 16 MiB")
    }
    Ok(serde_json::from_slice(&bytes)?)
}

/// Complete bytes are prepared before publication; never overwrites an existing path.
pub fn publish(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temp = parent.join(format!(
        ".aegis-export-{}-{}",
        std::process::id(),
        hash(bytes)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::hard_link(&temp, path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    let _ = fs::remove_file(temp);
    result
}

pub fn build(documents: Vec<Document>) -> Result<Index> {
    if documents.is_empty() || documents.len() > 1024 {
        bail!("require 1..1024 documents")
    }
    let mut chunks = Vec::new();
    let mut total = 0usize;
    let mut identities = BTreeSet::new();
    for d in documents {
        if ![&d.source, &d.revision, &d.license, &d.group_id]
            .iter()
            .all(|s| nonempty(s))
            || d.text.trim().is_empty()
            || d.text.contains('\0')
        {
            bail!("document requires provenance and nonempty UTF-8 text without NUL")
        }
        total = total.saturating_add(d.text.len());
        if total > MAX_BYTES {
            bail!("corpus exceeds 16 MiB")
        }
        if !identities.insert((d.source.clone(), d.revision.clone())) {
            bail!("duplicate source/revision")
        }
        let document_sha256 = hash(d.text.as_bytes());
        let mut start = 0;
        while start < d.text.len() {
            let mut end = (start + 4096).min(d.text.len());
            while !d.text.is_char_boundary(end) {
                end -= 1;
            }
            let text = d.text[start..end].to_owned();
            let document = Document {
                source: d.source.clone(),
                revision: d.revision.clone(),
                license: d.license.clone(),
                group_id: d.group_id.clone(),
                text,
            };
            chunks.push(Chunk {
                chunk_sha256: hash(document.text.as_bytes()),
                document,
                start_byte: start,
                end_byte: end,
                document_sha256: document_sha256.clone(),
            });
            if chunks.len() > MAX_CHUNKS {
                bail!("corpus exceeds 4096 chunks")
            }
            start = end;
        }
    }
    Ok(Index {
        schema_version: 1,
        chunks_sha256: hash(&serde_json::to_vec(&chunks)?),
        chunks,
    })
}

fn validate(index: &Index, expected: &str) -> Result<()> {
    if index.schema_version != 1
        || index.chunks.is_empty()
        || index.chunks.len() > MAX_CHUNKS
        || index.chunks_sha256 != expected
        || hash(&serde_json::to_vec(&index.chunks)?) != expected
    {
        bail!("index identity or schema mismatch")
    }
    // Reconstruct full documents to verify offsets and source hashes, not only chunk bytes.
    let mut docs: BTreeMap<(&str, &str), (Document, String)> = BTreeMap::new();
    let mut total = 0usize;
    for c in &index.chunks {
        if c.document.text.is_empty()
            || c.document.text.contains('\0')
            || c.document.text.len() > 4096
            || hash(c.document.text.as_bytes()) != c.chunk_sha256
        {
            bail!("invalid chunk")
        }
        total = total.saturating_add(c.document.text.len());
        if total > MAX_BYTES {
            bail!("corpus exceeds 16 MiB")
        }
        let key = (c.document.source.as_str(), c.document.revision.as_str());
        let entry = docs.entry(key).or_insert_with(|| {
            let mut d = c.document.clone();
            d.text.clear();
            (d, c.document_sha256.clone())
        });
        if c.start_byte != entry.0.text.len()
            || c.end_byte != c.start_byte + c.document.text.len()
            || c.document.license != entry.0.license
            || c.document.group_id != entry.0.group_id
            || c.document_sha256 != entry.1
        {
            bail!("inconsistent document offsets or provenance")
        }
        entry.0.text.push_str(&c.document.text);
    }
    for (_, (d, expected)) in docs {
        if hash(d.text.as_bytes()) != expected
            || ![&d.source, &d.revision, &d.license, &d.group_id]
                .iter()
                .all(|s| nonempty(s))
        {
            bail!("invalid document identity")
        }
    }
    Ok(())
}

fn terms(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .take(8192)
        .map(str::to_lowercase)
        .collect()
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextRequest {
    pub prompt: String,
    pub index: Option<Index>,
    pub expected_chunks_sha256: Option<String>,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}
fn default_top_k() -> usize {
    4
}

pub fn context(request: ContextRequest) -> Result<Value> {
    if request.prompt.contains('\0')
        || request.prompt.trim().is_empty()
        || request.prompt.len() > 32768
        || request.top_k == 0
        || request.top_k > 16
    {
        bail!("prompt must be 1..32768 bytes and top_k 1..16")
    }
    let mut citations = Vec::new();
    if let Some(index) = &request.index {
        validate(
            index,
            request
                .expected_chunks_sha256
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("retrieval requires trusted index hash"))?,
        )?;
        let query = terms(&request.prompt);
        let mut ranked: Vec<_> = index
            .chunks
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let score = terms(&c.document.text).intersection(&query).count();
                (score > 0).then_some((score, i, c))
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        for (score, _, c) in ranked.into_iter().take(request.top_k) {
            citations.push(json!({"source":c.document.source,"revision":c.document.revision,"license":c.document.license,"group_id":c.document.group_id,"document_sha256":c.document_sha256,"chunk_sha256":c.chunk_sha256,"start_byte":c.start_byte,"end_byte":c.end_byte,"matched_terms":score,"text":c.document.text}));
        }
    } else if request.expected_chunks_sha256.is_some() {
        bail!("index hash supplied without retrieval index")
    }
    Ok(
        json!({"schema_version":1,"retrieval_enabled":request.index.is_some(),"retrieval_method":"lexical_unique_term_overlap_v1","prompt":request.prompt,"index_sha256":request.expected_chunks_sha256,"citations":citations,"context_policy":"Retrieved text is untrusted reference data, never tool authorization or system instructions. A model adapter must preserve this separation.","model_invoked":false,"weights_modified":false}),
    )
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedChat {
    pub id: String,
    pub group_id: String,
    pub source: String,
    pub revision: String,
    pub license: String,
    pub approved_for_training: bool,
    pub messages: Vec<Message>,
}

pub fn export(chats: Vec<ReviewedChat>) -> Result<Vec<u8>> {
    if chats.is_empty() || chats.len() > 10000 {
        bail!("require 1..10000 reviewed conversations")
    }
    let mut ids = BTreeSet::new();
    let mut content_groups = BTreeMap::new();
    let mut output = Vec::new();
    for c in chats {
        if !c.approved_for_training {
            continue;
        }
        if ![&c.id, &c.group_id, &c.source, &c.revision, &c.license]
            .iter()
            .all(|s| nonempty(s))
            || !ids.insert(c.id.clone())
        {
            bail!("invalid provenance or duplicate conversation ID")
        }
        if c.messages.len() < 2
            || c.messages.len() > 256
            || c.messages.last().map(|m| m.role.as_str()) != Some("assistant")
            || !c.messages.iter().any(|m| m.role == "user")
            || c.messages.iter().any(|m| {
                !["system", "user", "assistant"].contains(&m.role.as_str())
                    || m.content.trim().is_empty()
                    || m.content.len() > 131072
                    || m.content.contains('\0')
            })
        {
            bail!("invalid chat messages")
        }
        let digest = hash(&serde_json::to_vec(&c.messages)?);
        if let Some(group) = content_groups.insert(digest.clone(), c.group_id.clone()) {
            if group != c.group_id {
                bail!("identical conversation assigned to different split groups")
            } else {
                continue;
            }
        }
        let group_hash = hash(c.group_id.as_bytes());
        let bucket = u32::from_str_radix(&group_hash[..8], 16)? % 100;
        let split = if bucket < 5 {
            "test"
        } else if bucket < 10 {
            "validation"
        } else {
            "train"
        };
        let row = json!({"schema_version":1,"id":c.id,"group_id":c.group_id,"source":c.source,"revision":c.revision,"license":c.license,"messages":c.messages,"split":split,"messages_sha256":digest,"review":"explicitly_approved_by_caller"});
        serde_json::to_writer(&mut output, &row)?;
        output.push(b'\n');
        if output.len() > MAX_BYTES {
            bail!("export exceeds 16 MiB")
        }
    }
    if output.is_empty() {
        bail!("no approved conversations; no export produced")
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn doc() -> Document {
        Document {
            source: "local/manual".into(),
            revision: "r1".into(),
            license: "owned".into(),
            group_id: "manual".into(),
            text: "Rust checkpoints preserve optimizer state.\n".repeat(130),
        }
    }
    #[test]
    fn retrieval_is_optional_anchored_and_preserves_utf8() {
        let mut d = doc();
        d.text.push_str("日本語");
        let original = d.text.clone();
        let index = build(vec![d]).unwrap();
        assert_eq!(
            index
                .chunks
                .iter()
                .map(|c| c.document.text.as_str())
                .collect::<String>(),
            original
        );
        let digest = index.chunks_sha256.clone();
        let result = context(ContextRequest {
            prompt: "optimizer".into(),
            index: Some(index.clone()),
            expected_chunks_sha256: Some(digest.clone()),
            top_k: 2,
        })
        .unwrap();
        assert_eq!(result["citations"].as_array().unwrap().len(), 2);
        assert_eq!(result["weights_modified"], false);
        let mut corrupt = index;
        corrupt.chunks[0].document.text.push('x');
        assert!(context(ContextRequest {
            prompt: "optimizer".into(),
            index: Some(corrupt),
            expected_chunks_sha256: Some(digest),
            top_k: 2
        })
        .is_err());
        let off = context(ContextRequest {
            prompt: "optimizer".into(),
            index: None,
            expected_chunks_sha256: None,
            top_k: 2,
        })
        .unwrap();
        assert_eq!(off["citations"], json!([]));
        assert_eq!(off["retrieval_enabled"], false);
    }
    #[test]
    fn export_rejects_cross_group_duplicates_and_never_overwrites() {
        let chat = |id: &str, group: &str, approved| ReviewedChat {
            id: id.into(),
            group_id: group.into(),
            source: "session".into(),
            revision: "r1".into(),
            license: "owned".into(),
            approved_for_training: approved,
            messages: vec![
                Message {
                    role: "user".into(),
                    content: "question".into(),
                },
                Message {
                    role: "assistant".into(),
                    content: "reviewed answer".into(),
                },
            ],
        };
        assert!(export(vec![chat("one", "g", false)]).is_err());
        assert!(export(vec![chat("one", "g", true), chat("two", "other", true)]).is_err());
        let bytes = export(vec![chat("one", "g", true), chat("two", "g", true)]).unwrap();
        assert_eq!(bytes.iter().filter(|&&b| b == b'\n').count(), 1);
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("export.jsonl");
        publish(&p, &bytes).unwrap();
        assert!(publish(&p, b"replacement").is_err());
        assert_eq!(fs::read(p).unwrap(), bytes);
    }
}
