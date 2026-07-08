use crate::message::{MessageDirection, MessageFilter, MessageSource, MessageStatus};

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_store() -> MessageStore {
        MessageStore::open_in_memory().unwrap()
    }

    #[test]
    fn inserts_and_lists_messages_newest_first() {
        let store = memory_store();
        let first = store
            .insert_message(NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+15551234567".to_string(),
                body: "hello".to_string(),
                timestamp: "2026-07-08T12:00:00Z".to_string(),
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: Some("/org/freedesktop/ModemManager1/SMS/1".to_string()),
                read_at: None,
                error: None,
            })
            .unwrap();
        let second = store.insert_message(NewMessage::inbound("+15550000000", "later")).unwrap();

        let rows = store.list_messages(&MessageFilter::default()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, second.id);
        assert_eq!(rows[1].id, first.id);
        assert_eq!(rows[1].body, "hello");
        assert_eq!(rows[1].modem_sms_path.as_deref(), Some("/org/freedesktop/ModemManager1/SMS/1"));
    }

    #[test]
    fn filters_search_unread_direction_status_and_phone() {
        let store = memory_store();
        store.insert_message(NewMessage::inbound("+1", "alpha code")).unwrap();
        store
            .insert_message(NewMessage {
                direction: MessageDirection::Outbound,
                phone_number: "+1".to_string(),
                body: "alpha reply".to_string(),
                timestamp: "2026-07-08T12:01:00Z".to_string(),
                status: MessageStatus::Sent,
                source: MessageSource::Web,
                modem_sms_path: None,
                read_at: Some("2026-07-08T12:01:00Z".to_string()),
                error: None,
            })
            .unwrap();
        store.insert_message(NewMessage::inbound("+2", "beta")).unwrap();

        let rows = store
            .list_messages(&MessageFilter {
                phone_number: Some("+1".to_string()),
                q: Some("alpha".to_string()),
                direction: Some(MessageDirection::Inbound),
                status: Some(MessageStatus::Received),
                unread: Some(true),
                ..MessageFilter::default()
            })
            .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].phone_number, "+1");
        assert_eq!(rows[0].direction, MessageDirection::Inbound);
        assert_eq!(rows[0].status, MessageStatus::Received);
        assert!(rows[0].read_at.is_none());
    }

    #[test]
    fn marks_single_message_and_conversation_read_unread() {
        let store = memory_store();
        let one = store.insert_message(NewMessage::inbound("+1", "one")).unwrap();
        let two = store.insert_message(NewMessage::inbound("+1", "two")).unwrap();
        store.insert_message(NewMessage::inbound("+2", "other")).unwrap();

        store.mark_read(one.id).unwrap();
        assert!(store.get_message(one.id).unwrap().read_at.is_some());
        store.mark_unread(one.id).unwrap();
        assert!(store.get_message(one.id).unwrap().read_at.is_none());

        let changed = store.mark_conversation_read("+1").unwrap();
        assert_eq!(changed, 2);
        assert!(store.get_message(one.id).unwrap().read_at.is_some());
        assert!(store.get_message(two.id).unwrap().read_at.is_some());

        let unread = store
            .list_messages(&MessageFilter {
                unread: Some(true),
                ..MessageFilter::default()
            })
            .unwrap();
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].phone_number, "+2");
    }

    #[test]
    fn deletes_multiple_messages() {
        let store = memory_store();
        let one = store.insert_message(NewMessage::inbound("+1", "one")).unwrap();
        let two = store.insert_message(NewMessage::inbound("+2", "two")).unwrap();
        store.delete_messages(&[one.id, two.id]).unwrap();
        assert!(store.list_messages(&MessageFilter::default()).unwrap().is_empty());
    }

    #[test]
    fn conversations_include_last_message_and_unread_counts() {
        let store = memory_store();
        store.insert_message(NewMessage::inbound("+1", "old unread")).unwrap();
        let latest = store.insert_message(NewMessage::inbound("+1", "new unread")).unwrap();
        let read = store.insert_message(NewMessage::inbound("+2", "read")).unwrap();
        store.mark_read(read.id).unwrap();

        let conversations = store.list_conversations().unwrap();
        assert_eq!(conversations.len(), 2);
        assert_eq!(conversations[0].phone_number, "+1");
        assert_eq!(conversations[0].last_message.id, latest.id);
        assert_eq!(conversations[0].unread_count, 2);
        assert_eq!(conversations[0].total_count, 2);
        assert_eq!(conversations[1].phone_number, "+2");
        assert_eq!(conversations[1].unread_count, 0);
    }

    #[test]
    fn export_ignores_page_limit_and_uses_stable_csv_columns() {
        let store = memory_store();
        store.insert_message(NewMessage::inbound("+1", "alpha")).unwrap();
        store.insert_message(NewMessage::inbound("+1", "alpha second")).unwrap();

        let filter = MessageFilter {
            limit: Some(1),
            q: Some("alpha".to_string()),
            ..MessageFilter::default()
        };
        let rows = store.export_messages(&filter).unwrap();
        assert_eq!(rows.len(), 2);

        let csv = store.export_messages_csv(&filter).unwrap();
        assert!(csv.starts_with("id,direction,phone_number,body,timestamp,status,source,read_at,error,created_at,updated_at\n"));
        assert!(csv.contains("alpha second"));
    }
}
