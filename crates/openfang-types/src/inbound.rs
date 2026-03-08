use crate::media::{MediaSource, MediaType};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Attachment received from an ingress channel before it is imported into managed storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundAttachment {
    pub kind: MediaType,
    pub mime_type: String,
    pub filename: Option<String>,
    pub source: MediaSource,
    pub size_bytes: u64,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Lifetime scope for a managed attachment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentScope {
    Turn,
    Session,
    Agent,
}

/// Attachment managed by the kernel attachment store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAttachment {
    pub id: String,
    pub kind: MediaType,
    pub mime_type: String,
    pub filename: Option<String>,
    pub stored_path: String,
    pub size_bytes: u64,
    pub source_channel: String,
    pub scope: AttachmentScope,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Normalized rich inbound message passed into the kernel.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InboundMessage {
    pub text: Option<String>,
    #[serde(default)]
    pub attachments: Vec<InboundAttachment>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}
