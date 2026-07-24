use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

#[cfg(test)]
use crate::events::AppEvent;
use crate::export::MessageExportFormat;
use crate::message::{
    Message, MessageCursor, MessageDirection, MessageFilter, MessageSource, MessageStatus,
};
use crate::messaging::SendMessage;
#[cfg(test)]
use crate::storage::NewMessage;

use super::{ApiError, ApiResult, ApiState};

#[derive(Deserialize)]
pub struct SendRequest {
    phone_number: String,
    body: String,
}

#[derive(Deserialize)]
pub struct DeleteManyRequest {
    ids: Vec<i64>,
}

#[derive(Default, Deserialize)]
pub struct MessageQuery {
    limit: Option<u32>,
    before_timestamp: Option<String>,
    before_id: Option<i64>,
    phone_number: Option<String>,
    q: Option<String>,
    direction: Option<MessageDirection>,
    status: Option<MessageStatus>,
    unread: Option<bool>,
    from: Option<String>,
    to: Option<String>,
    format: Option<String>,
}

fn to_filter(q: &MessageQuery) -> ApiResult<MessageFilter> {
    let before = match (&q.before_timestamp, q.before_id) {
        (Some(timestamp), Some(id)) => Some(MessageCursor::Timeline {
            timestamp: timestamp.clone(),
            id,
        }),
        (None, None) => None,
        (None, Some(id)) => Some(MessageCursor::LegacyId(id)),
        (Some(_), None) => {
            return Err(ApiError::bad_request(
                "before_id is required when before_timestamp is provided",
            ));
        }
    };
    Ok(MessageFilter {
        limit: q.limit,
        before,
        phone_number: q.phone_number.clone(),
        q: q.q.clone(),
        direction: q.direction,
        status: q.status,
        unread: q.unread,
        from: q.from.clone(),
        to: q.to.clone(),
    })
}

pub fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/messages", get(list_messages))
        .route("/api/conversations", get(list_conversations))
        .route("/api/messages/send", post(send_message))
        .route("/api/messages/{id}/read", post(mark_read))
        .route("/api/messages/{id}/unread", post(mark_unread))
        .route(
            "/api/conversations/{phone_number}/read",
            post(mark_conversation_read),
        )
        .route("/api/messages/{id}", delete(delete_message))
        .route("/api/messages/delete", post(delete_many))
        .route("/api/messages/export", get(export_messages))
        .route("/api/events", get(events))
}

async fn list_messages(
    State(state): State<ApiState>,
    Query(query): Query<MessageQuery>,
) -> ApiResult<Json<Vec<Message>>> {
    let filter = to_filter(&query)?;
    let rows = state.messaging().list(filter).await.map_err(|error| {
        if error
            .downcast_ref::<crate::storage::InvalidMessageCursor>()
            .is_some()
        {
            ApiError::bad_request(error.to_string())
        } else {
            ApiError::internal(error.to_string())
        }
    })?;
    Ok(Json(rows))
}

async fn list_conversations(
    State(state): State<ApiState>,
) -> ApiResult<Json<Vec<crate::message::ConversationSummary>>> {
    let rows = state
        .messaging()
        .conversations()
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;
    Ok(Json(rows))
}

async fn send_message(
    State(state): State<ApiState>,
    Json(req): Json<SendRequest>,
) -> ApiResult<Json<Message>> {
    if req.phone_number.trim().is_empty() {
        return Err(ApiError::bad_request("phone_number is required"));
    }
    if req.body.trim().is_empty() {
        return Err(ApiError::bad_request("body is required"));
    }
    let phone_number = req.phone_number.trim().to_string();
    let body = req.body;
    let updated = state
        .messaging()
        .send(SendMessage {
            modem_path: state.config.app.modem_path.clone(),
            phone_number: phone_number.clone(),
            body: body.clone(),
            source: MessageSource::Web,
        })
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?
        .into_message();
    Ok(Json(updated))
}

async fn mark_read(State(state): State<ApiState>, Path(id): Path<i64>) -> ApiResult<Json<Message>> {
    let msg = state
        .messaging()
        .set_read(id, true)
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;
    Ok(Json(msg))
}

async fn mark_unread(
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Message>> {
    let msg = state
        .messaging()
        .set_read(id, false)
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;
    Ok(Json(msg))
}

async fn mark_conversation_read(
    State(state): State<ApiState>,
    Path(phone_number): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let changed = state
        .messaging()
        .mark_conversation_read(phone_number)
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;
    Ok(Json(serde_json::json!({ "changed": changed })))
}

async fn delete_message(
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    state
        .messaging()
        .delete(vec![id])
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_many(
    State(state): State<ApiState>,
    Json(req): Json<DeleteManyRequest>,
) -> ApiResult<StatusCode> {
    let ids = req.ids.clone();
    state
        .messaging()
        .delete(ids)
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn export_messages(
    State(state): State<ApiState>,
    Query(query): Query<MessageQuery>,
) -> ApiResult<Response> {
    let format = if query.format.as_deref() == Some("json") {
        MessageExportFormat::Json
    } else {
        MessageExportFormat::Csv
    };
    let filter = to_filter(&query)?;
    let stream = crate::export::stream(&state.store, filter, format);

    let body = Body::from_stream(stream);
    let headers = match format {
        MessageExportFormat::Json => [
            (
                header::CONTENT_TYPE,
                "application/json; charset=utf-8".to_string(),
            ),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=messages.json".to_string(),
            ),
        ],
        MessageExportFormat::Csv => [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=messages.csv".to_string(),
            ),
        ],
    };
    Ok((headers, body).into_response())
}

async fn events(State(state): State<ApiState>) -> impl IntoResponse {
    let rx = state.events.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(event) => match serde_json::to_string(&event) {
            Ok(data) => Some(Ok::<_, std::convert::Infallible>(
                Event::default().event(event.name()).data(data),
            )),
            Err(_) => None,
        },
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone, Default)]
    struct RecordingSmsSender {
        calls: Arc<Mutex<Vec<(String, String, String)>>>,
    }

    impl crate::dbus::SmsSender for RecordingSmsSender {
        fn send<'a>(
            &'a self,
            modem_path: &'a str,
            tel_number: &'a str,
            sms_text: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<crate::dbus::SendSmsOutcome>> + Send + 'a>>
        {
            let calls = self.calls.clone();
            let modem_path = modem_path.to_string();
            let tel_number = tel_number.to_string();
            let sms_text = sms_text.to_string();
            Box::pin(async move {
                calls
                    .lock()
                    .unwrap()
                    .push((modem_path, tel_number, sms_text));
                Ok(crate::dbus::SendSmsOutcome {
                    modem_sms_path: "/org/freedesktop/ModemManager1/SMS/test".to_string(),
                })
            })
        }
    }

    #[test]
    fn routes_build_without_panicking() {
        let _ = super::routes();
    }

    #[test]
    fn timeline_cursor_requires_timestamp_and_id_together() {
        assert!(to_filter(&MessageQuery {
            before_id: Some(42),
            ..MessageQuery::default()
        })
        .is_ok());
        assert!(to_filter(&MessageQuery {
            before_timestamp: Some("2026-07-19T12:00:00Z".to_string()),
            ..MessageQuery::default()
        })
        .is_err());
    }

    #[tokio::test]
    async fn send_message_uses_the_state_sms_sender() {
        let sender = RecordingSmsSender::default();
        let store = crate::storage::MessageStore::open_in_memory().unwrap();
        let state = super::super::ApiState {
            config: std::sync::Arc::new(crate::config::AppConfig::default()),
            config_path: std::path::PathBuf::from("/tmp/not-used.toml"),
            store: store.clone().into(),
            events: crate::events::EventBus::new(),
            delivery_wakeup: crate::delivery::DeliveryWakeup::new(),
            started_at: std::time::Instant::now(),
            sessions: super::super::auth::SessionStore::default(),
            modem: crate::modem::ModemService::new(),
            sms_sender: Arc::new(sender.clone()),
        };

        let message = send_message(
            State(state),
            Json(SendRequest {
                phone_number: "+15551234567".to_string(),
                body: "test body".to_string(),
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(message.status, MessageStatus::Sent);
        assert_eq!(
            sender.calls.lock().unwrap().as_slice(),
            [(
                "/org/freedesktop/ModemManager1/Modem/0".to_string(),
                "+15551234567".to_string(),
                "test body".to_string(),
            )]
        );
        assert_eq!(
            store.get_message(message.id).unwrap().status,
            MessageStatus::Sent
        );
    }

    #[tokio::test]
    async fn deleted_legacy_cursor_returns_bad_request() {
        use tower::ServiceExt;

        let store = crate::storage::MessageStore::open_in_memory().unwrap();
        store
            .insert_message(NewMessage::inbound("+1", "older"))
            .unwrap();
        let cursor = store
            .insert_message(NewMessage::inbound("+1", "cursor"))
            .unwrap();
        store.delete_messages(&[cursor.id]).unwrap();
        let state = super::super::ApiState {
            config: std::sync::Arc::new(crate::config::AppConfig::default()),
            config_path: std::path::PathBuf::from("/tmp/not-used.toml"),
            store: store.into(),
            events: crate::events::EventBus::new(),
            delivery_wakeup: crate::delivery::DeliveryWakeup::new(),
            started_at: std::time::Instant::now(),
            sessions: super::super::auth::SessionStore::default(),
            modem: crate::modem::ModemService::new(),
            sms_sender: super::super::test_sms_sender(),
        };
        let app = routes().with_state(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/api/messages?before_id={}", cursor.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn streams_json_and_csv_without_collecting_the_result_set() {
        let store = crate::storage::MessageStore::open_in_memory().unwrap();
        store
            .insert_message(NewMessage::inbound("+1", "first"))
            .unwrap();
        store
            .insert_message(NewMessage::inbound("+2", "second"))
            .unwrap();

        for format in [MessageExportFormat::Json, MessageExportFormat::Csv] {
            let worker_store = crate::persistence::Store::from(store.clone());
            let mut stream = crate::export::stream(&worker_store, MessageFilter::default(), format);
            let mut bytes = Vec::new();
            while let Some(chunk) = stream.next().await {
                bytes.extend_from_slice(&chunk.unwrap());
            }

            match format {
                MessageExportFormat::Json => {
                    let messages: Vec<Message> = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(messages.len(), 2);
                }
                MessageExportFormat::Csv => {
                    let csv = String::from_utf8(bytes).unwrap();
                    assert!(csv.starts_with("id,direction,phone_number,body"));
                    assert!(csv.contains("first"));
                    assert!(csv.contains("second"));
                }
            }
        }
    }

    #[tokio::test]
    async fn bulk_conversation_read_emits_one_event() {
        let store = crate::storage::MessageStore::open_in_memory().unwrap();
        for i in 0..50 {
            store
                .insert_message(NewMessage::inbound("+1", &format!("unread-{i}")))
                .unwrap();
        }
        let events = crate::events::EventBus::new();
        let mut receiver = events.subscribe();
        let state = super::super::ApiState {
            config: std::sync::Arc::new(crate::config::AppConfig::default()),
            config_path: std::path::PathBuf::from("/tmp/not-used.toml"),
            store: store.into(),
            events,
            delivery_wakeup: crate::delivery::DeliveryWakeup::new(),
            started_at: std::time::Instant::now(),
            sessions: super::super::auth::SessionStore::default(),
            modem: crate::modem::ModemService::new(),
            sms_sender: super::super::test_sms_sender(),
        };

        let response = mark_conversation_read(State(state), Path("+1".to_string()))
            .await
            .unwrap();

        assert_eq!(response.0["changed"], 50);
        assert!(matches!(
            receiver.recv().await.unwrap(),
            AppEvent::ConversationRead
        ));
        assert!(receiver.try_recv().is_err());
    }
}
