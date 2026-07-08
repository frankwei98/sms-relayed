use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tokio_stream::wrappers::BroadcastStream;
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
        .route("/api/messages/:id/read", post(mark_read))
        .route("/api/messages/:id/unread", post(mark_unread))
        .route(
            "/api/conversations/:phone_number/read",
            post(mark_conversation_read),
        )
        .route("/api/messages/:id", delete(delete_message))
        .route("/api/messages/delete", post(delete_many))
        .route("/api/messages/export", get(export_messages))
        .route("/api/events", get(events))
}

async fn list_messages(
    State(state): State<ApiState>,
    Query(query): Query<MessageQuery>,
) -> ApiResult<Json<Vec<Message>>> {
    let rows = state.store.list_messages(&to_filter(&query))?;
    Ok(Json(rows))
}

async fn list_conversations(
    State(state): State<ApiState>,
) -> ApiResult<Json<Vec<crate::message::ConversationSummary>>> {
    let rows = state.store.list_conversations()?;
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
    let changed = state.store.mark_conversation_read(&phone_number)?;
    let rows = state.store.list_messages(&MessageFilter {
        phone_number: Some(phone_number),
        ..MessageFilter::default()
    })?;
    for msg in &rows {
        state
            .events
            .send(AppEvent::MessageReadStateChanged(msg.clone()));
    }
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
    let messages = state.store.export_messages(&to_filter(&query))?;
    if query.format.as_deref() == Some("json") {
        return Ok(Json(messages).into_response());
    }
    let csv = state.store.export_messages_csv(&to_filter(&query))?;
    let headers = [
        (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
        (
            header::CONTENT_DISPOSITION,
            "attachment; filename=messages.csv".to_string(),
        ),
    ];
    Ok((headers, csv).into_response())
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
