use prost::Message;
use std::collections::HashMap;

/// Include generated protobuf structures
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/ox_workflow.rs"));
}

/// Enum for representing fields in memory.
/// String: for simple text values (status codes, paths, headers).
/// Bytes: for binary-encoded structured data (protobuf messages from plugins).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    String(String),
    Bytes(Vec<u8>),
}

/// In-memory abstraction wrapping the protobuf-derived HashMap
#[derive(Debug, Default, Clone)]
pub struct TaskState {
    pub fields: HashMap<String, FieldValue>,
}

impl TaskState {
    pub fn new() -> Self {
        Self {
            fields: HashMap::new(),
        }
    }

    /// Serializes the in-memory TaskState to protobuf bytes for persistence
    pub fn to_proto_bytes(&self) -> Vec<u8> {
        let mut proto_state = proto::TaskState::default();
        for (k, v) in &self.fields {
            let any = match v {
                FieldValue::String(s) => prost_types::Any {
                    type_url: "type.googleapis.com/google.protobuf.StringValue".to_string(),
                    value: s.as_bytes().to_vec(),
                },
                FieldValue::Bytes(b) => prost_types::Any {
                    type_url: "type.googleapis.com/google.protobuf.BytesValue".to_string(),
                    value: b.clone(),
                },
            };
            proto_state.fields.insert(k.clone(), any);
        }
        proto_state.encode_to_vec()
    }

    /// Deserializes the protobuf bytes into the in-memory TaskState
    pub fn from_proto_bytes(bytes: &[u8]) -> Result<Self, prost::DecodeError> {
        let proto_state = proto::TaskState::decode(bytes)?;
        let mut fields = HashMap::new();
        for (k, v) in proto_state.fields {
            if v.type_url.ends_with("BytesValue") {
                fields.insert(k, FieldValue::Bytes(v.value));
            } else if let Ok(s) = String::from_utf8(v.value) {
                fields.insert(k, FieldValue::String(s));
            }
        }
        Ok(Self { fields })
    }
}
