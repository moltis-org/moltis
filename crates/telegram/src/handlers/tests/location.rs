use super::*;

#[test]
fn extract_location_from_message() {
    let msg: Message = serde_json::from_value(json!({
        "message_id": 1,
        "date": 1,
        "chat": { "id": 42, "type": "private", "first_name": "Alice" },
        "from": {
            "id": 1001,
            "is_bot": false,
            "first_name": "Alice",
            "username": "alice"
        },
        "location": {
            "latitude": 48.8566,
            "longitude": 2.3522
        }
    }))
    .expect("deserialize location message");

    let loc = extract_location(&msg);
    assert!(loc.is_some(), "should extract location from message");
    let info = loc.unwrap();
    assert!((info.latitude - 48.8566).abs() < 1e-4);
    assert!((info.longitude - 2.3522).abs() < 1e-4);
    assert!(!info.is_live, "static location should not be live");
}

#[test]
fn extract_location_returns_none_for_text() {
    let msg: Message = serde_json::from_value(json!({
        "message_id": 1,
        "date": 1,
        "chat": { "id": 42, "type": "private", "first_name": "Alice" },
        "from": {
            "id": 1001,
            "is_bot": false,
            "first_name": "Alice"
        },
        "text": "hello"
    }))
    .expect("deserialize text message");

    assert!(extract_location(&msg).is_none());
}

#[test]
fn location_messages_are_marked_with_location_message_kind() {
    let msg: Message = serde_json::from_value(json!({
        "message_id": 1,
        "date": 1,
        "chat": { "id": 42, "type": "private", "first_name": "Alice" },
        "from": {
            "id": 1001,
            "is_bot": false,
            "first_name": "Alice"
        },
        "location": {
            "latitude": 48.8566,
            "longitude": 2.3522
        }
    }))
    .expect("deserialize location message");

    assert!(matches!(
        message_kind(&msg),
        Some(ChannelMessageKind::Location)
    ));
}

#[test]
fn extract_location_detects_live_period() {
    let msg: Message = serde_json::from_value(json!({
        "message_id": 1,
        "date": 1,
        "chat": { "id": 42, "type": "private", "first_name": "Alice" },
        "from": {
            "id": 1001,
            "is_bot": false,
            "first_name": "Alice"
        },
        "location": {
            "latitude": 48.8566,
            "longitude": 2.3522,
            "live_period": 3600
        }
    }))
    .expect("deserialize live location message");

    let info = extract_location(&msg).expect("should extract live location");
    assert!(info.is_live, "location with live_period should be live");
    assert!((info.latitude - 48.8566).abs() < 1e-4);
}
