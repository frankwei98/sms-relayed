use serde::Serialize;
use tokio::sync::broadcast;

use crate::message::Message;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum AppEvent {
    MessageCreated(Message),
    MessageUpdated(Message),
    MessageDeleted { ids: Vec<i64> },
    MessageReadStateChanged(Message),
    ConversationRead,
    ConfigSaved,
    ServiceRestartScheduled,
}

#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<AppEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(256);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.sender.subscribe()
    }

    pub fn send(&self, event: AppEvent) {
        let _ = self.sender.send(event);
    }
}

impl AppEvent {
    pub fn name(&self) -> &'static str {
        match self {
            AppEvent::MessageCreated(_) => "message.created",
            AppEvent::MessageUpdated(_) => "message.updated",
            AppEvent::MessageDeleted { .. } => "message.deleted",
            AppEvent::MessageReadStateChanged(_) => "message.read_state_changed",
            AppEvent::ConversationRead => "conversation.read",
            AppEvent::ConfigSaved => "config.saved",
            AppEvent::ServiceRestartScheduled => "service.restart_scheduled",
        }
    }
}
