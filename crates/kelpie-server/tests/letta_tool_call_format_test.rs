// Unit test to verify tool_call format for Letta SDK compatibility
//
// Tests that tool_call serializes to JSON with proper field names and types

use chrono::Utc;
use kelpie_server::models::{LettaToolCall, Message, MessageRole};

#[test]
fn test_letta_tool_call_serialization() {
    // Create a LettaToolCall
    let tool_call = LettaToolCall {
        name: "echo".to_string(),
        arguments: r#"{"input": "hello"}"#.to_string(),
        tool_call_id: "call_123".to_string(),
    };

    // Serialize to JSON
    let json = serde_json::to_value(&tool_call).unwrap();

    // Verify fields are accessible as object properties
    assert_eq!(json["name"], "echo");
    assert_eq!(json["arguments"], r#"{"input": "hello"}"#);
    assert_eq!(json["tool_call_id"], "call_123");

    // Verify no extra fields
    assert!(json.is_object());
    let obj = json.as_object().unwrap();
    assert_eq!(obj.len(), 3, "Should have exactly 3 fields");
}

#[test]
fn test_message_with_tool_call() {
    // Create a Message with a tool_call (Letta format)
    let message = Message {
        id: "msg_1".to_string(),
        agent_id: "agent_1".to_string(),
        message_type: "tool_call_message".to_string(),
        role: MessageRole::Assistant,
        content: "Calling echo tool".to_string(),
        tool_call_id: None,
        tool_calls: vec![],
        tool_call: Some(LettaToolCall {
            name: "echo".to_string(),
            arguments: r#"{"input": "test"}"#.to_string(),
            tool_call_id: "call_456".to_string(),
        }),
        tool_return: None,
        status: None,
        created_at: Utc::now(),
    };

    // Serialize to JSON
    let json = serde_json::to_value(&message).unwrap();

    // Verify tool_call field exists and is an object (not null)
    assert!(
        json["tool_call"].is_object(),
        "tool_call should be an object"
    );

    // Verify tool_call properties are accessible (Letta SDK does: m.tool_call.name)
    assert_eq!(json["tool_call"]["name"], "echo");
    assert_eq!(json["tool_call"]["arguments"], r#"{"input": "test"}"#);
    assert_eq!(json["tool_call"]["tool_call_id"], "call_456");
}

#[test]
fn test_message_without_tool_call() {
    // Create a Message WITHOUT a tool_call
    let message = Message {
        id: "msg_2".to_string(),
        agent_id: "agent_1".to_string(),
        message_type: "assistant_message".to_string(),
        role: MessageRole::Assistant,
        content: "Hello".to_string(),
        tool_call_id: None,
        tool_calls: vec![],
        tool_call: None,
        tool_return: None,
        status: None,
        created_at: Utc::now(),
    };

    // Serialize to JSON
    let json = serde_json::to_value(&message).unwrap();

    // Verify tool_call field is omitted (not serialized when None)
    // This is important for clean API responses
    assert!(
        !json.as_object().unwrap().contains_key("tool_call"),
        "tool_call should be omitted when None (skip_serializing_if)"
    );
}
