use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct IdempotencyConflict;

impl std::fmt::Display for IdempotencyConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("idempotency key was already used with a different request")
    }
}

impl std::error::Error for IdempotencyConflict {}

#[derive(Debug)]
pub struct IdempotencyReplayUnavailable;

impl std::fmt::Display for IdempotencyReplayUnavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(
            "idempotency key was already completed, but its message is no longer available",
        )
    }
}

impl std::error::Error for IdempotencyReplayUnavailable {}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageStatus {
    Received,
    Sending,
    Sent,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageSource {
    Modem,
    Web,
    Cli,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub id: i64,
    pub direction: MessageDirection,
    pub phone_number: String,
    pub body: String,
    pub timestamp: String,
    pub status: MessageStatus,
    pub source: MessageSource,
    pub modem_sms_path: Option<String>,
    pub read_at: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MessageFilter {
    pub limit: Option<u32>,
    pub before: Option<MessageCursor>,
    pub phone_number: Option<String>,
    pub q: Option<String>,
    pub direction: Option<MessageDirection>,
    pub status: Option<MessageStatus>,
    pub unread: Option<bool>,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub enum MessageCursor {
    Timeline { timestamp: String, id: i64 },
    LegacyId(i64),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConversationSummary {
    pub phone_number: String,
    pub last_message: Message,
    pub unread_count: i64,
    pub total_count: i64,
}
