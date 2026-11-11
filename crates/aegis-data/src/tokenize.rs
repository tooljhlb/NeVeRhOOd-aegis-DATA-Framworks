//! Bounded document-local byte windows exported as NumPy int64 arrays.
use crate::{ingest::hash, wire::Sample};
use anyhow::{bail, Result};
use prost::Message;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, Read, Write},
    path::Path,
};
pub fn export(input: &Path, out: &Path, tokens: usize, max_rows: usize) -> Result<usize> {
    if !(2..=256).contains(&tokens)
        || max_rows == 0
        || max_rows
            .checked_mul(tokens)
            .is_none_or(|n| n > 8 * 1024 * 1024)
    {
        bail!("tokens must be 2..256 and total budget <=8M int64 values")
    }
    if max_rows > 65536 {
        bail!("row budget exceeds 65536")
    }
    let mut f = BufReader::new(File::open(input)?);
    let mut reservoir = std::collections::BinaryHeap::new();
    loop {
        let mut size = [0u8; 4];
        if f.read(&mut size[..1])? == 0 {
            break;
        }
        f.read_exact(&mut size[1..])?;
        let len = u32::from_le_bytes(size) as usize;
        if len > 4 * 1024 * 1024 + 1024 {
            bail!("oversize frame")
        }
        let mut bytes = vec![0; len];
        f.read_exact(&mut bytes)?;
        let s = Sample::decode(bytes.as_slice())?;
        if s.text.is_empty() || hash(s.text.as_bytes()) != s.sha256 {
            bail!("invalid text record")
        }
        let group = hash(s.group_id.as_bytes());
        for (index, chunk) in s.text.as_bytes().chunks_exact(tokens).enumerate() {
            let priority = hash(format!("{}:{index}", s.sha256).as_bytes());
            let row = (
                priority,
                s.sha256.clone(),
                group.clone(),
                index,
                chunk.iter().map(|&b| i64::from(b) + 4).collect::<Vec<_>>(),
            );
            if reservoir.len() < max_rows {
                reservoir.push(row)
            } else if reservoir.peek().is_some_and(|top| row < *top) {
                reservoir.pop();
                reservoir.push(row)
            }
        }
    }
    let mut selected = reservoir.into_vec();
    selected.sort_by(|a, b| a.1.cmp(&b.1).then(a.3.cmp(&b.3)));
    let rows = selected.len();
    if rows == 0 {
        bail!("no complete document-local windows; reduce token count")
    }
    let row_documents: Vec<_> = selected.iter().map(|r| r.1.clone()).collect();
    let row_groups: Vec<_> = selected.iter().map(|r| r.2.clone()).collect();
    let row_offsets: Vec<_> = selected.iter().map(|r| r.3 * tokens).collect();
    let values: Vec<i64> = selected.into_iter().flat_map(|r| r.4).collect();
    let mut header =
        format!("{{'descr': '<i8', 'fortran_order': False, 'shape': ({rows}, {tokens}), }}");
    let padding = (64 - ((10 + header.len() + 1) % 64)) % 64;
    header.push_str(&" ".repeat(padding));
    header.push('\n');
    let len = u16::try_from(header.len())?;
    let mut target = OpenOptions::new().write(true).create_new(true).open(out)?;
    target.write_all(b"\x93NUMPY\x01\x00")?;
    target.write_all(&len.to_le_bytes())?;
    target.write_all(header.as_bytes())?;
    for v in values {
        target.write_all(&v.to_le_bytes())?
    }
    target.sync_all()?;
    fn file_hash(path: &Path) -> Result<String> {
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
    let manifest = serde_json::json!({"schema_version":2,"row_documents":row_documents,"row_groups":row_groups,"row_offsets":row_offsets,"window_sampling":"sha256_priority_reservoir_v1","input_shard_sha256":file_hash(input)?,"output_sha256":file_hash(out)?,"rows":rows,"tokens_per_row":tokens,"tokenizer":"utf8-byte-plus4-v1","document_policy":"discard_incomplete_tail_no_document_join"});
    let mut manifest_path = out.as_os_str().to_owned();
    manifest_path.push(".manifest.json");
    let mut marker = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(std::path::PathBuf::from(manifest_path))?;
    marker.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    marker.sync_all()?;
    Ok(rows)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_npy_header_and_values() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("text.pb");
        let out = d.path().join("text.npy");
        let s = Sample {
            sha256: hash(b"abcd"),
            text: "abcd".into(),
            source: "fixture".into(),
            license: "CC0".into(),
            group_id: "g".into(),
            split: "train".into(),
            revision: "v1".into(),
            ..Default::default()
        };
        let p = s.encode_to_vec();
        let mut f = File::create(&src).unwrap();
        f.write_all(&(p.len() as u32).to_le_bytes()).unwrap();
        f.write_all(&p).unwrap();
        drop(f);
        assert_eq!(export(&src, &out, 2, 10).unwrap(), 2);
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(&bytes[..8], b"\x93NUMPY\x01\x00");
        let h = u16::from_le_bytes(bytes[8..10].try_into().unwrap()) as usize;
        assert_eq!(
            i64::from_le_bytes(bytes[10 + h..18 + h].try_into().unwrap()),
            101
        );
        assert!(export(&src, &out, 2, 10).is_err());
    }
}
