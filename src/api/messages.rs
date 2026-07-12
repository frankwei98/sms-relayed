use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

use crate::events::AppEvent;
use crate::message::{Message, MessageDirection, MessageFilter, MessageSource, MessageStatus};
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

#[derive(Deserialize)]
pub struct MessageQuery {
    limit: Option<u32>,
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

fn to_filter(q: &MessageQuery) -> MessageFilter {
    MessageFilter {
        limit: q.limit,
        before_id: q.before_id,
        phone_number: q.phone_number.clone(),
        q: q.q.clone(),
        direction: q.direction,
        status: q.status,
        unread: q.unread,
        from: q.from.clone(),
        to: q.to.clone(),
    }
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
    let store = state.store.clone();
    let filter = to_filter(&query);
    let rows = tokio::task::spawn_blocking(move || store.list_messages(&filter))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))??;
    Ok(Json(rows))
}

async fn list_conversations(
    State(state): State<ApiState>,
) -> ApiResult<Json<Vec<crate::message::ConversationSummary>>> {
    let store = state.store.clone();
    let rows = tokio::task::spawn_blocking(move || store.list_conversations())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))??;
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
    let now = now_string();
    let new = NewMessage {
        direction: MessageDirection::Outbound,
        phone_number: req.phone_number.trim().to_string(),
        body: req.body.clone(),
        timestamp: now.clone(),
        status: MessageStatus::Sending,
        source: MessageSource::Web,
        modem_sms_path: None,
        read_at: Some(now),
        error: None,
        inbound_dedupe_key: None,
    };
    let msg = state.store.insert_message(new)?;
    state.events.send(AppEvent::MessageCreated(msg.clone()));

    match crate::dbus::send_sms_via_system(
        &state.config.app.modem_path,
        &req.phone_number,
        &req.body,
    )
    .await
    {
        Ok(_outcome) => {
            let updated = state
                .store
                .update_status(msg.id, MessageStatus::Sent, None)?;
            state.events.send(AppEvent::MessageUpdated(updated.clone()));
            Ok(Json(updated))
        }
        Err(err) => {
            let updated =
                state
                    .store
                    .update_status(msg.id, MessageStatus::Failed, Some(err.to_string()))?;
            state.events.send(AppEvent::MessageUpdated(updated.clone()));
            Ok(Json(updated))
        }
    }
}

async fn mark_read(State(state): State<ApiState>, Path(id): Path<i64>) -> ApiResult<Json<Message>> {
    let msg = state.store.mark_read(id)?;
    state
        .events
        .send(AppEvent::MessageReadStateChanged(msg.clone()));
    Ok(Json(msg))
}

async fn mark_unread(
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Message>> {
    let msg = state.store.mark_unread(id)?;
    state
        .events
        .send(AppEvent::MessageReadStateChanged(msg.clone()));
    Ok(Json(msg))
}

async fn mark_conversation_read(
    State(state): State<ApiState>,
    Path(phone_number): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let store = state.store.clone();
    let changed = tokio::task::spawn_blocking(move || store.mark_conversation_read(&phone_number))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))??;
    state.events.send(AppEvent::ConversationRead);
    Ok(Json(serde_json::json!({ "changed": changed })))
}

async fn delete_message(
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    state.store.delete_messages(&[id])?;
    state
        .events
        .send(AppEvent::MessageDeleted { ids: vec![id] });
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_many(
    State(state): State<ApiState>,
    Json(req): Json<DeleteManyRequest>,
) -> ApiResult<StatusCode> {
    let ids = req.ids.clone();
    state.store.delete_messages(&ids)?;
    state.events.send(AppEvent::MessageDeleted { ids });
    Ok(StatusCode::NO_CONTENT)
}

async fn export_messages(
    State(state): State<ApiState>,
    Query(query): Query<MessageQuery>,
) -> ApiResult<Response> {
    let format = if query.format.as_deref() == Some("json") {
        ExportFormat::Json
    } else {
        ExportFormat::Csv
    };
    let filter = to_filter(&query);
    let store = state.store.clone();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(8);

    tokio::task::spawn_blocking(move || {
        let result = stream_export(&store, &filter, format, &tx);
        if let Err(err) = result {
            let _ = tx.blocking_send(Err(std::io::Error::other(err.to_string())));
        }
    });

    let body = Body::from_stream(ReceiverStream::new(rx));
    let headers = match format {
        ExportFormat::Json => [
            (
                header::CONTENT_TYPE,
                "application/json; charset=utf-8".to_string(),
            ),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=messages.json".to_string(),
            ),
        ],
        ExportFormat::Csv => [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=messages.csv".to_string(),
            ),
        ],
    };
    Ok((headers, body).into_response())
}

#[derive(Clone, Copy)]
enum ExportFormat {
    Json,
    Csv,
}

fn stream_export(
    store: &crate::storage::MessageStore,
    filter: &MessageFilter,
    format: ExportFormat,
    tx: &tokio::sync::mpsc::Sender<Result<Bytes, std::io::Error>>,
) -> anyhow::Result<()> {
    match format {
        ExportFormat::Json => {
            if tx.blocking_send(Ok(Bytes::from_static(b"["))).is_err() {
                return Ok(());
            }
            let mut first = true;
            store.for_each_export_message(filter, |message| {
                let mut chunk = if first { Vec::new() } else { vec![b','] };
                first = false;
                serde_json::to_writer(&mut chunk, &message)?;
                Ok(tx.blocking_send(Ok(Bytes::from(chunk))).is_ok())
            })?;
            let _ = tx.blocking_send(Ok(Bytes::from_static(b"]")));
        }
        ExportFormat::Csv => {
            let header = csv_record_bytes(&[
                "id",
                "direction",
                "phone_number",
                "body",
                "timestamp",
                "status",
                "source",
                "read_at",
                "error",
                "created_at",
                "updated_at",
            ])?;
            if tx.blocking_send(Ok(Bytes::from(header))).is_err() {
                return Ok(());
            }
            store.for_each_export_message(filter, |message| {
                let fields = vec![
                    message.id.to_string(),
                    enum_json(&message.direction)?,
                    message.phone_number,
                    message.body,
                    message.timestamp,
                    enum_json(&message.status)?,
                    enum_json(&message.source)?,
                    message.read_at.unwrap_or_default(),
                    message.error.unwrap_or_default(),
                    message.created_at,
                    message.updated_at,
                ];
                let chunk = csv_record_bytes(&fields)?;
                Ok(tx.blocking_send(Ok(Bytes::from(chunk))).is_ok())
            })?;
        }
    }
    Ok(())
}

fn csv_record_bytes<S: AsRef<str>>(fields: &[S]) -> anyhow::Result<Vec<u8>> {
    let mut writer = csv::WriterBuilder::new()
        .terminator(csv::Terminator::Any(b'\n'))
        .from_writer(Vec::new());
    writer.write_record(fields.iter().map(AsRef::as_ref))?;
    Ok(writer.into_inner()?)
}

fn enum_json<T: serde::Serialize>(value: &T) -> anyhow::Result<String> {
    let encoded = serde_json::to_string(value)?;
    Ok(encoded.trim_matches('"').to_string())
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

fn now_string() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_build_without_panicking() {
        let _ = super::routes();
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

        for format in [ExportFormat::Json, ExportFormat::Csv] {
            let (tx, mut rx) = tokio::sync::mpsc::channel(1);
            let worker_store = store.clone();
            let worker = tokio::task::spawn_blocking(move || {
                stream_export(&worker_store, &MessageFilter::default(), format, &tx)
            });
            let mut bytes = Vec::new();
            while let Some(chunk) = rx.recv().await {
                bytes.extend_from_slice(&chunk.unwrap());
            }
            worker.await.unwrap().unwrap();

            match format {
                ExportFormat::Json => {
                    let messages: Vec<Message> = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(messages.len(), 2);
                }
                ExportFormat::Csv => {
                    let csv = String::from_utf8(bytes).unwrap();
                    assert!(csv.starts_with("id,direction,phone_number,body"));
                    assert!(csv.contains("first"));
                    assert!(csv.contains("second"));
                }
            }
        }
    }

    #[tokio::test]
    async fn export_stops_when_client_disconnects() {
        let store = crate::storage::MessageStore::open_in_memory().unwrap();
        for i in 0..100 {
            store
                .insert_message(NewMessage::inbound("+1", &format!("message-{i}")))
                .unwrap();
        }
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(rx);
        let worker = tokio::task::spawn_blocking(move || {
            stream_export(&store, &MessageFilter::default(), ExportFormat::Json, &tx)
        });
        tokio::time::timeout(std::time::Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
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
            store,
            events,
            started_at: std::time::Instant::now(),
            sessions: super::super::auth::SessionStore::default(),
            modem: crate::modem::ModemService::new(),
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
