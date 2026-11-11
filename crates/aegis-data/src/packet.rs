//! Versioned binary interchange with raw 32-byte content IDs. JSON stays at edges.
use anyhow::{bail, Result};
use prost::Message;
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};
const LIMIT: usize = 16 * 1024 * 1024;
const MAGIC: &[u8; 4] = b"AEG1";
#[derive(Clone, PartialEq, Message)]
pub struct Packet {
    #[prost(uint32, tag = "1")]
    pub schema_version: u32,
    #[prost(enumeration = "Encoding", tag = "2")]
    pub encoding: i32,
    #[prost(enumeration = "Topic", tag = "3")]
    pub topic: i32,
    #[prost(bytes = "vec", tag = "4")]
    pub content_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "5")]
    pub payload: Vec<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub enum Encoding {
    Unspecified = 0,
    Protobuf = 1,
    Flatbuffers = 2,
    Raw = 3,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub enum Topic {
    Unspecified = 0,
    Artifact = 1,
    TrainingBatch = 2,
    Telemetry = 3,
    ToolEvent = 4,
}
fn validate(packet: &Packet) -> Result<()> {
    if packet.schema_version != 1
        || packet.payload.is_empty()
        || packet.payload.len() > LIMIT
        || packet.content_id.len() != 32
        || !matches!(
            Encoding::try_from(packet.encoding),
            Ok(Encoding::Protobuf | Encoding::Flatbuffers | Encoding::Raw)
        )
        || !matches!(
            Topic::try_from(packet.topic),
            Ok(Topic::Artifact | Topic::TrainingBatch | Topic::Telemetry | Topic::ToolEvent)
        )
    {
        bail!("invalid binary packet contract")
    }
    if packet.content_id != Sha256::digest(&packet.payload).as_slice() {
        bail!("packet content ID mismatch")
    }
    Ok(())
}
pub fn pack(payload: Vec<u8>, encoding: Encoding, topic: Topic) -> Result<Vec<u8>> {
    let packet = Packet {
        schema_version: 1,
        encoding: encoding as i32,
        topic: topic as i32,
        content_id: Sha256::digest(&payload).to_vec(),
        payload,
    };
    validate(&packet)?;
    let encoded = packet.encode_to_vec();
    let mut frame = Vec::with_capacity(encoded.len() + 8);
    frame.extend(MAGIC);
    frame.extend((encoded.len() as u32).to_le_bytes());
    frame.extend(encoded);
    Ok(frame)
}
pub fn unpack(frame: &[u8]) -> Result<Packet> {
    if frame.len() < 8 || frame.len() > LIMIT + 1024 || &frame[..4] != MAGIC {
        bail!("invalid packet magic or size")
    }
    let count = u32::from_le_bytes(frame[4..8].try_into()?) as usize;
    if count != frame.len() - 8 {
        bail!("truncated packet or trailing bytes")
    }
    let packet = Packet::decode(&frame[8..])?;
    validate(&packet)?;
    Ok(packet)
}
pub fn read(path: &Path, framed: bool) -> Result<Vec<u8>> {
    let max = LIMIT + if framed { 1024 } else { 0 };
    let mut bytes = Vec::new();
    File::open(path)?
        .take((max + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > max {
        bail!("packet input exceeds bound")
    };
    Ok(bytes)
}
pub fn summary(packet: &Packet) -> serde_json::Value {
    serde_json::json!({"schema_version":packet.schema_version,"encoding":packet.encoding,"topic":packet.topic,"payload_bytes":packet.payload.len(),"content_id_hex":crate::ingest::hash(&packet.payload),"content_id_wire_bytes":packet.content_id.len(),"payload_schema_verified":false})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_ids_round_trip_and_corruption_fail_closed() {
        let bytes = b"\0raw\xffbytes".to_vec();
        let frame = pack(bytes.clone(), Encoding::Raw, Topic::Artifact).unwrap();
        let packet = unpack(&frame).unwrap();
        assert_eq!(packet.payload, bytes);
        assert_eq!(packet.content_id.len(), 32);
        let mut bad = frame.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(unpack(&bad).is_err());
        bad = frame.clone();
        bad.push(0);
        assert!(unpack(&bad).is_err());
        assert!(unpack(&frame[..frame.len() - 1]).is_err());
        assert!(pack(vec![], Encoding::Raw, Topic::Artifact).is_err());
        let mut bad_packet = packet;
        bad_packet.content_id.pop();
        let body = bad_packet.encode_to_vec();
        let mut bad = MAGIC.to_vec();
        bad.extend((body.len() as u32).to_le_bytes());
        bad.extend(body);
        assert!(unpack(&bad).is_err());
    }
}