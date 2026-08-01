use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
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
    ConversationDeleteBlocked, ConversationNotFound, Message, MessageCursor, MessageDirection,
    MessageFilter, MessageNotFound, MessageSource, MessageStatus,
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

fn map_message_resource_error(error: anyhow::Error) -> ApiError {
    if error.downcast_ref::<MessageNotFound>().is_some() {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "message_not_found",
            error.to_string(),
        )
    } else {
        ApiError::internal(error.to_string())
    }
}

fn map_conversation_resource_error(error: anyhow::Error) -> ApiError {
    if error.downcast_ref::<ConversationNotFound>().is_some() {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "conversation_not_found",
            error.to_string(),
        )
    } else if error.downcast_ref::<ConversationDeleteBlocked>().is_some() {
        ApiError::new(
            StatusCode::CONFLICT,
            "conversation_delete_blocked",
            error.to_string(),
        )
    } else {
        ApiError::internal(error.to_string())
    }
}

pub fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/messages", get(list_messages))
        .route("/api/messages/favorites", get(list_favorites))
        .route("/api/conversations", get(list_conversations))
        .route("/api/messages/send", post(send_message))
        .route("/api/messages/{id}/read", post(mark_read))
        .route("/api/messages/{id}/unread", post(mark_unread))
        .route("/api/messages/{id}/favorite", post(favorite_message))
        .route("/api/messages/{id}/unfavorite", post(unfavorite_message))
        .route(
            "/api/conversations/{phone_number}/read",
            post(mark_conversation_read),
        )
        .route(
            "/api/conversations/{phone_number}/pin",
            post(pin_conversation),
        )
        .route(
            "/api/conversations/{phone_number}/unpin",
            post(unpin_conversation),
        )
        .route(
            "/api/conversations/{phone_number}",
            delete(delete_conversation),
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

async fn list_favorites(State(state): State<ApiState>) -> ApiResult<Json<Vec<Message>>> {
    let rows = state
        .messaging()
        .favorites()
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;
    Ok(Json(rows))
}

async fn send_message(
    State(state): State<ApiState>,
    headers: HeaderMap,
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
    let value = headers.get("idempotency-key").ok_or_else(|| {
        ApiError::new(
            StatusCode::PRECONDITION_REQUIRED,
            "idempotency_key_required",
            "Idempotency-Key header is required",
        )
    })?;
    let value = value
        .to_str()
        .map_err(|_| ApiError::bad_request("invalid Idempotency-Key header"))?
        .trim();
    if value.is_empty() || value.len() > 200 {
        return Err(ApiError::bad_request(
            "Idempotency-Key must contain 1 to 200 characters",
        ));
    }
    let idempotency_key = Some(value.to_string());
    let updated = state
        .messaging()
        .send(SendMessage {
            modem_path: state.config.app.modem_path.clone(),
            phone_number: phone_number.clone(),
            body: body.clone(),
            source: MessageSource::Web,
            idempotency_key,
        })
        .await
        .map_err(|error| {
            if error
                .downcast_ref::<crate::message::IdempotencyConflict>()
                .is_some()
            {
                ApiError::new(
                    StatusCode::CONFLICT,
                    "idempotency_conflict",
                    error.to_string(),
                )
            } else if error
                .downcast_ref::<crate::message::IdempotencyReplayUnavailable>()
                .is_some()
            {
                ApiError::new(
                    StatusCode::CONFLICT,
                    "idempotency_replay_unavailable",
                    error.to_string(),
                )
            } else {
                ApiError::internal(error.to_string())
            }
        })?
        .into_message();
    Ok(Json(updated))
}

async fn mark_read(State(state): State<ApiState>, Path(id): Path<i64>) -> ApiResult<Json<Message>> {
    let msg = state
        .messaging()
        .set_read(id, true)
        .await
        .map_err(map_message_resource_error)?;
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
        .map_err(map_message_resource_error)?;
    Ok(Json(msg))
}

async fn favorite_message(
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Message>> {
    let message = state
        .messaging()
        .set_favorite(id, true)
        .await
        .map_err(map_message_resource_error)?;
    Ok(Json(message))
}

async fn unfavorite_message(
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Message>> {
    let message = state
        .messaging()
        .set_favorite(id, false)
        .await
        .map_err(map_message_resource_error)?;
    Ok(Json(message))
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

async fn pin_conversation(
    State(state): State<ApiState>,
    Path(phone_number): Path<String>,
) -> ApiResult<StatusCode> {
    state
        .messaging()
        .set_conversation_pinned(phone_number, true)
        .await
        .map_err(map_conversation_resource_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unpin_conversation(
    State(state): State<ApiState>,
    Path(phone_number): Path<String>,
) -> ApiResult<StatusCode> {
    state
        .messaging()
        .set_conversation_pinned(phone_number, false)
        .await
        .map_err(map_conversation_resource_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_conversation(
    State(state): State<ApiState>,
    Path(phone_number): Path<String>,
) -> ApiResult<StatusCode> {
    state
        .messaging()
        .delete_conversation(phone_number)
        .await
        .map_err(map_conversation_resource_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_message(
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    state
        .messaging()
        .delete(vec![id])
        .await
        .map_err(map_message_resource_error)?;
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
        .map_err(map_message_resource_error)?;
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
        fn prepare<'a>(
            &'a self,
            modem_path: &'a str,
            tel_number: &'a str,
            sms_text: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<crate::dbus::PreparedSms>> + Send + 'a>>
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
                Ok(crate::dbus::PreparedSms {
                    modem_sms_path: "/org/freedesktop/ModemManager1/SMS/test".to_string(),
                })
            })
        }

        fn send_prepared<'a>(
            &'a self,
            _modem_sms_path: &'a str,
        ) -> Pin<Box<dyn Future<Output = crate::dbus::SendAttemptOutcome> + Send + 'a>> {
            Box::pin(async { crate::dbus::SendAttemptOutcome::Accepted })
        }

        fn sms_state<'a>(
            &'a self,
            _modem_sms_path: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<crate::dbus::ModemSmsState>> + Send + 'a>>
        {
            Box::pin(async { Ok(crate::dbus::ModemSmsState::Sent) })
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
        let modem = crate::modem::ModemService::new();
        modem.set_verified_path(Some("/org/freedesktop/ModemManager1/Modem/0".to_string()));
        let state = super::super::ApiState {
            config: std::sync::Arc::new(crate::config::AppConfig::default()),
            config_path: std::path::PathBuf::from("/tmp/not-used.toml"),
            config_save_lock: Arc::new(tokio::sync::Mutex::new(())),
            store: store.clone().into(),
            events: crate::events::EventBus::new(),
            delivery_wakeup: crate::delivery::DeliveryWakeup::new(),
            started_at: std::time::Instant::now(),
            sessions: super::super::auth::SessionStore::default(),
            modem,
            sms_sender: Arc::new(sender.clone()),
            service_control: super::super::service::ServiceControl::default(),
        };

        let missing_key = send_message(
            State(state.clone()),
            HeaderMap::new(),
            Json(SendRequest {
                phone_number: "+15551234567".to_string(),
                body: "test body".to_string(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(missing_key.status, StatusCode::PRECONDITION_REQUIRED);

        let mut headers = HeaderMap::new();
        headers.insert("idempotency-key", "api-request-1".parse().unwrap());
        let message = send_message(
            State(state.clone()),
            headers.clone(),
            Json(SendRequest {
                phone_number: "+15551234567".to_string(),
                body: "test body".to_string(),
            }),
        )
        .await
        .unwrap()
        .0;
        let repeated = send_message(
            State(state.clone()),
            headers.clone(),
            Json(SendRequest {
                phone_number: "+15551234567".to_string(),
                body: "test body".to_string(),
            }),
        )
        .await
        .unwrap()
        .0;
        let conflict = send_message(
            State(state),
            headers,
            Json(SendRequest {
                phone_number: "+15551234567".to_string(),
                body: "different body".to_string(),
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(message.status, MessageStatus::Sent);
        assert_eq!(repeated.id, message.id);
        assert_eq!(conflict.status, StatusCode::CONFLICT);
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
            config_save_lock: Arc::new(tokio::sync::Mutex::new(())),
            store: store.into(),
            events: crate::events::EventBus::new(),
            delivery_wakeup: crate::delivery::DeliveryWakeup::new(),
            started_at: std::time::Instant::now(),
            sessions: super::super::auth::SessionStore::default(),
            modem: crate::modem::ModemService::new(),
            sms_sender: super::super::test_sms_sender(),
            service_control: super::super::service::ServiceControl::default(),
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
            config_save_lock: Arc::new(tokio::sync::Mutex::new(())),
            store: store.into(),
            events,
            delivery_wakeup: crate::delivery::DeliveryWakeup::new(),
            started_at: std::time::Instant::now(),
            sessions: super::super::auth::SessionStore::default(),
            modem: crate::modem::ModemService::new(),
            sms_sender: super::super::test_sms_sender(),
            service_control: super::super::service::ServiceControl::default(),
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

    #[tokio::test]
    async fn missing_message_and_conversation_resources_return_not_found() {
        use tower::ServiceExt;

        let store = crate::storage::MessageStore::open_in_memory().unwrap();
        let state = super::super::ApiState {
            config: std::sync::Arc::new(crate::config::AppConfig::default()),
            config_path: std::path::PathBuf::from("/tmp/not-used.toml"),
            config_save_lock: Arc::new(tokio::sync::Mutex::new(())),
            store: store.into(),
            events: crate::events::EventBus::new(),
            delivery_wakeup: crate::delivery::DeliveryWakeup::new(),
            started_at: std::time::Instant::now(),
            sessions: super::super::auth::SessionStore::default(),
            modem: crate::modem::ModemService::new(),
            sms_sender: super::super::test_sms_sender(),
            service_control: super::super::service::ServiceControl::default(),
        };
        let app = routes().with_state(state);

        for (method, uri) in [
            (axum::http::Method::POST, "/api/messages/404/favorite"),
            (
                axum::http::Method::POST,
                "/api/conversations/%2B15550000404/pin",
            ),
            (
                axum::http::Method::DELETE,
                "/api/conversations/%2B15550000404",
            ),
            (axum::http::Method::DELETE, "/api/messages/404"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
        }
    }

    #[tokio::test]
    async fn favorites_and_pins_are_persistent_api_resources() {
        use tower::ServiceExt;

        let store = crate::storage::MessageStore::open_in_memory().unwrap();
        let first = store
            .insert_message(NewMessage::inbound("+1", "favorite me"))
            .unwrap();
        store
            .insert_message(NewMessage::inbound("+2", "newest conversation"))
            .unwrap();
        let state = super::super::ApiState {
            config: std::sync::Arc::new(crate::config::AppConfig::default()),
            config_path: std::path::PathBuf::from("/tmp/not-used.toml"),
            config_save_lock: Arc::new(tokio::sync::Mutex::new(())),
            store: store.into(),
            events: crate::events::EventBus::new(),
            delivery_wakeup: crate::delivery::DeliveryWakeup::new(),
            started_at: std::time::Instant::now(),
            sessions: super::super::auth::SessionStore::default(),
            modem: crate::modem::ModemService::new(),
            sms_sender: super::super::test_sms_sender(),
            service_control: super::super::service::ServiceControl::default(),
        };
        let app = routes().with_state(state);

        let favorite_response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/api/messages/{}/favorite", first.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(favorite_response.status(), StatusCode::OK);

        let pin_response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/conversations/%2B1/pin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(pin_response.status(), StatusCode::NO_CONTENT);

        let favorites_response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/messages/favorites")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(favorites_response.status(), StatusCode::OK);
        let favorites_body = axum::body::to_bytes(favorites_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let favorites: serde_json::Value = serde_json::from_slice(&favorites_body).unwrap();
        assert_eq!(favorites[0]["id"], first.id);
        assert!(favorites[0]["favorite_at"].is_string());

        let conversations_response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/conversations")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let conversations_body =
            axum::body::to_bytes(conversations_response.into_body(), usize::MAX)
                .await
                .unwrap();
        let conversations: serde_json::Value = serde_json::from_slice(&conversations_body).unwrap();
        assert_eq!(conversations[0]["phone_number"], "+1");
        assert_eq!(conversations[0]["pinned"], true);
        assert_eq!(conversations[0]["favorite_count"], 1);
        assert_eq!(conversations[0]["delete_blocked"], false);
    }

    #[tokio::test]
    async fn deleting_a_conversation_removes_its_messages_favorites_and_pin() {
        use tower::ServiceExt;

        let store = crate::storage::MessageStore::open_in_memory().unwrap();
        let message = store
            .insert_message(NewMessage::inbound("+1", "delete together"))
            .unwrap();
        let state = super::super::ApiState {
            config: std::sync::Arc::new(crate::config::AppConfig::default()),
            config_path: std::path::PathBuf::from("/tmp/not-used.toml"),
            config_save_lock: Arc::new(tokio::sync::Mutex::new(())),
            store: store.into(),
            events: crate::events::EventBus::new(),
            delivery_wakeup: crate::delivery::DeliveryWakeup::new(),
            started_at: std::time::Instant::now(),
            sessions: super::super::auth::SessionStore::default(),
            modem: crate::modem::ModemService::new(),
            sms_sender: super::super::test_sms_sender(),
            service_control: super::super::service::ServiceControl::default(),
        };
        let app = routes().with_state(state);

        for uri in [
            format!("/api/messages/{}/favorite", message.id),
            "/api/conversations/%2B1/pin".to_string(),
        ] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(response.status().is_success());
        }

        let delete_response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("DELETE")
                    .uri("/api/conversations/%2B1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

        for uri in ["/api/conversations", "/api/messages/favorites"] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let rows: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(rows.as_array().unwrap().len(), 0);
        }
    }

    #[tokio::test]
    async fn deleting_a_conversation_with_a_sending_message_returns_conflict() {
        use tower::ServiceExt;

        let store = crate::storage::MessageStore::open_in_memory().unwrap();
        store
            .insert_message(NewMessage {
                direction: MessageDirection::Outbound,
                phone_number: "+1".to_string(),
                body: "still sending".to_string(),
                timestamp: "2026-08-01T00:00:00Z".to_string(),
                status: MessageStatus::Sending,
                source: MessageSource::Web,
                modem_sms_path: None,
                read_at: Some("2026-08-01T00:00:00Z".to_string()),
                error: None,
                inbound_dedupe_key: None,
            })
            .unwrap();
        let state = super::super::ApiState {
            config: std::sync::Arc::new(crate::config::AppConfig::default()),
            config_path: std::path::PathBuf::from("/tmp/not-used.toml"),
            config_save_lock: Arc::new(tokio::sync::Mutex::new(())),
            store: store.into(),
            events: crate::events::EventBus::new(),
            delivery_wakeup: crate::delivery::DeliveryWakeup::new(),
            started_at: std::time::Instant::now(),
            sessions: super::super::auth::SessionStore::default(),
            modem: crate::modem::ModemService::new(),
            sms_sender: super::super::test_sms_sender(),
            service_control: super::super::service::ServiceControl::default(),
        };

        let response = routes()
            .with_state(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("DELETE")
                    .uri("/api/conversations/%2B1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
    }
}
