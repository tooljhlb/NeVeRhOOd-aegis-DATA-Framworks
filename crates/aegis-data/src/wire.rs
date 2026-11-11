use prost::Message;
// Wire-compatible with schemas/sample.proto. Payload frame is u32 LE length + protobuf.
#[derive(Clone, PartialEq, Message)]
pub struct Sample {
    #[prost(string, tag = "1")]
    pub sha256: String,
    #[prost(string, tag = "2")]
    pub source: String,
    #[prost(string, tag = "3")]
    pub license: String,
    #[prost(string, tag = "4")]
    pub group_id: String,
    #[prost(string, tag = "5")]
    pub split: String,
    #[prost(string, tag = "6")]
    pub text: String,
    #[prost(string, tag = "7")]
    pub revision: String,
    #[prost(string, tag = "8")]
    pub content_kind: String,
    #[prost(string, tag = "9")]
    pub normalization: String,
    #[prost(string, tag = "10")]
    pub original_sha256: String,
}
