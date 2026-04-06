use serde::{Deserialize, Serialize};

/// 浏览器 → nexus-gateway
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum BrowserInbound {
    #[serde(rename = "message")]
    Message { content: String, #[serde(default)] media: Option<Vec<String>> },
    #[serde(rename = "new_session")]
    NewSession,
    #[serde(rename = "switch_session")]
    SwitchSession { session_id: String },
}

/// nexus-gateway → 浏览器
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum BrowserOutbound {
    #[serde(rename = "message")]
    Message { content: String, session_id: String, #[serde(skip_serializing_if = "Option::is_none")] media: Option<Vec<String>> },
    #[serde(rename = "progress")]
    Progress { content: String, session_id: String },
    #[serde(rename = "error")]
    Error { reason: String },
    #[serde(rename = "session_created")]
    SessionCreated { session_id: String },
    #[serde(rename = "session_switched")]
    SessionSwitched { session_id: String },
}

/// nexus-server → nexus-gateway（通过 /ws/nexus）
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NexusInbound {
    Auth { token: String },
    Send { chat_id: String, content: String, metadata: Option<serde_json::Value> },
}

/// nexus-gateway → nexus-server（通过 /ws/nexus）
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NexusOutbound {
    AuthOk,
    AuthFail { reason: String },
    Message { chat_id: String, sender_id: String, content: String, session_id: String, #[serde(skip_serializing_if = "Option::is_none")] media: Option<Vec<String>> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_inbound_deserializes() {
        let json = r#"{"type":"message","content":"hello"}"#;
        let msg: BrowserInbound = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, BrowserInbound::Message { content, media } if content == "hello" && media.is_none()));
    }

    #[test]
    fn browser_inbound_with_media_deserializes() {
        let json = r#"{"type":"message","content":"hello","media":["file1:test.png"]}"#;
        let msg: BrowserInbound = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, BrowserInbound::Message { content, media } if content == "hello" && media.as_ref().unwrap().len() == 1));
    }

    #[test]
    fn nexus_inbound_auth_deserializes() {
        let json = r#"{"type":"auth","token":"secret"}"#;
        let msg: NexusInbound = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, NexusInbound::Auth { token } if token == "secret"));
    }

    #[test]
    fn nexus_inbound_send_deserializes() {
        let json = r#"{"type":"send","chat_id":"abc","content":"hi"}"#;
        let msg: NexusInbound = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, NexusInbound::Send { chat_id, content, metadata }
            if chat_id == "abc" && content == "hi" && metadata.is_none()));
    }

    #[test]
    fn nexus_inbound_send_with_metadata_deserializes() {
        let json = r#"{"type":"send","chat_id":"abc","content":"hi","metadata":{"_progress":true}}"#;
        let msg: NexusInbound = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, NexusInbound::Send { chat_id, content, metadata }
            if chat_id == "abc" && content == "hi" && metadata.is_some()));
    }

    #[test]
    fn browser_inbound_new_session_deserializes() {
        let json = r#"{"type":"new_session"}"#;
        let msg: BrowserInbound = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, BrowserInbound::NewSession));
    }

    #[test]
    fn browser_inbound_switch_session_deserializes() {
        let json = r#"{"type":"switch_session","session_id":"s1"}"#;
        let msg: BrowserInbound = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, BrowserInbound::SwitchSession { session_id } if session_id == "s1"));
    }

    #[test]
    fn nexus_outbound_message_serializes() {
        let msg = NexusOutbound::Message {
            chat_id: "abc".to_string(),
            sender_id: "user1".to_string(),
            content: "hello".to_string(),
            session_id: "gateway:user1:test".to_string(),
            media: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"message""#));
        assert!(json.contains(r#""chat_id":"abc""#));
        // media should be omitted when None
        assert!(!json.contains("media"));
    }

    #[test]
    fn nexus_outbound_message_with_media_serializes() {
        let msg = NexusOutbound::Message {
            chat_id: "abc".to_string(),
            sender_id: "user1".to_string(),
            content: "hello".to_string(),
            session_id: "gateway:user1:test".to_string(),
            media: Some(vec!["file1:test.png".to_string()]),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("media"));
        assert!(json.contains("file1:test.png"));
    }

    #[test]
    fn browser_outbound_error_serializes() {
        let msg = BrowserOutbound::Error { reason: "not connected".to_string() };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"error""#));
        assert!(json.contains("not connected"));
    }
}
