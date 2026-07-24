use std::io;
use std::pin::Pin;

use futures_util::stream::{self, Stream};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

use crate::message::{Message, MessageFilter};
use crate::persistence::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageExportFormat {
    Json,
    Csv,
}

pub type MessageExportStream = Pin<Box<dyn Stream<Item = io::Result<Vec<u8>>> + Send + 'static>>;

pub fn stream(
    store: &Store,
    filter: MessageFilter,
    format: MessageExportFormat,
) -> MessageExportStream {
    let mut first = true;
    let rows = ReceiverStream::new(store.stream_messages(filter, move |message| match format {
        MessageExportFormat::Json => encode_json_row(message, &mut first),
        MessageExportFormat::Csv => encode_csv_row(message),
    }))
    .map(|result| result.map_err(|error| io::Error::other(error.to_string())));

    match format {
        MessageExportFormat::Json => Box::pin(
            stream::once(async { Ok(vec![b'[']) })
                .chain(rows)
                .chain(stream::once(async { Ok(vec![b']']) })),
        ),
        MessageExportFormat::Csv => {
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
            ])
            .map_err(|error| io::Error::other(error.to_string()));
            Box::pin(stream::once(async move { header }).chain(rows))
        }
    }
}

fn encode_json_row(message: Message, first: &mut bool) -> anyhow::Result<Vec<u8>> {
    let mut chunk = if *first { Vec::new() } else { vec![b','] };
    *first = false;
    serde_json::to_writer(&mut chunk, &message)?;
    Ok(chunk)
}

fn encode_csv_row(message: Message) -> anyhow::Result<Vec<u8>> {
    csv_record_bytes(&[
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
    ])
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
