use serde::{Deserialize, Serialize};

/// 浏览器 → webui-server
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserInbound {
    Message { content: String },
}

/// webui-server → 浏览器
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserOutbound {
    Connected,
    Message { content: String },
    Error { reason: String },
}

/// nexus-server → webui-server（通过 /ws/nexus）
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NexusInbound {
    Auth { token: String },
    Send { chat_id: String, content: String },
}

/// webui-server → nexus-server（通过 /ws/nexus）
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NexusOutbound {
    AuthOk,
    AuthFail { reason: String },
    Message { chat_id: String, sender_id: String, content: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_inbound_deserializes() {
        let json = r#"{"type":"message","content":"hello"}"#;
        let msg: BrowserInbound = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, BrowserInbound::Message { content } if content == "hello"));
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
        assert!(matches!(msg, NexusInbound::Send { chat_id, content }
            if chat_id == "abc" && content == "hi"));
    }

    #[test]
    fn nexus_outbound_message_serializes() {
        let msg = NexusOutbound::Message {
            chat_id: "abc".to_string(),
            sender_id: "user1".to_string(),
            content: "hello".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"message""#));
        assert!(json.contains(r#""chat_id":"abc""#));
    }

    #[test]
    fn browser_outbound_error_serializes() {
        let msg = BrowserOutbound::Error { reason: "not connected".to_string() };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"error""#));
        assert!(json.contains("not connected"));
    }
}
