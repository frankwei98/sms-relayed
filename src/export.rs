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
    let mut fields = [
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
    for field in &mut fields {
        neutralize_spreadsheet_formula(field);
    }
    csv_record_bytes(&fields)
}

fn neutralize_spreadsheet_formula(field: &mut String) {
    if matches!(
        field.as_bytes().first(),
        Some(b'=' | b'+' | b'-' | b'@' | b'\t' | b'\r')
    ) {
        field.insert(0, '\'');
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{MessageDirection, MessageSource, MessageStatus};

    #[test]
    fn csv_export_neutralizes_spreadsheet_formula_prefixes() {
        let row = encode_csv_row(Message {
            id: 42,
            direction: MessageDirection::Inbound,
            phone_number: "=1+1".to_string(),
            body: "+SUM(A1:A2)".to_string(),
            timestamp: "-1+1".to_string(),
            status: MessageStatus::Received,
            source: MessageSource::Modem,
            modem_sms_path: None,
            read_at: Some("\t=1+1".to_string()),
            error: Some("@SUM(A1:A2)".to_string()),
            created_at: "\r=1+1".to_string(),
            updated_at: "2026-07-25T12:00:00Z".to_string(),
        })
        .unwrap();

        let record = csv::ReaderBuilder::new()
            .has_headers(false)
            .from_reader(row.as_slice())
            .records()
            .next()
            .unwrap()
            .unwrap();

        assert_eq!(&record[2], "'=1+1");
        assert_eq!(&record[3], "'+SUM(A1:A2)");
        assert_eq!(&record[4], "'-1+1");
        assert_eq!(&record[7], "'\t=1+1");
        assert_eq!(&record[8], "'@SUM(A1:A2)");
        assert_eq!(&record[9], "'\r=1+1");
        assert_eq!(&record[10], "2026-07-25T12:00:00Z");
    }
}
