//! Discord Gateway protocol types and event parsing.

use serde::Deserialize;
use serde_json::Value;

pub const OP_DISPATCH: u8 = 0;
pub const OP_HEARTBEAT: u8 = 1;
pub const OP_IDENTIFY: u8 = 2;
pub const OP_RECONNECT: u8 = 7;
pub const OP_INVALID_SESSION: u8 = 9;
pub const OP_HELLO: u8 = 10;
pub const OP_HEARTBEAT_ACK: u8 = 11;

pub const INTENTS: u64 = (1 << 0) | (1 << 9) | (1 << 12) | (1 << 15);

pub const GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=10&encoding=json";

#[derive(Debug, Deserialize)]
pub struct GatewayFrame {
    pub op: u8,
    pub d: Option<Value>,
    pub s: Option<u64>,
    pub t: Option<String>,
}

pub fn heartbeat_frame(seq: Option<u64>) -> String {
    serde_json::json!({
        "op": OP_HEARTBEAT,
        "d": seq
    }).to_string()
}

pub fn identify_frame(token: &str) -> String {
    serde_json::json!({
        "op": OP_IDENTIFY,
        "d": {
            "token": token,
            "intents": INTENTS,
            "properties": {
                "os": "nexus",
                "browser": "nexus",
                "device": "nexus"
            }
        }
    }).to_string()
}

pub struct ReadyData {
    pub bot_user_id: String,
}

pub fn parse_ready(d: &Value) -> Option<ReadyData> {
    let user_id = d.get("user")?.get("id")?.as_str()?;
    Some(ReadyData {
        bot_user_id: user_id.to_string(),
    })
}

#[derive(Debug)]
pub struct MessageCreateData {
    pub channel_id: String,
    pub guild_id: Option<String>,
    pub thread_id: Option<String>,
    pub sender_id: String,
    pub sender_name: String,
    pub sender_is_bot: bool,
    pub content: String,
    pub mentions: Vec<String>,
    pub attachments: Vec<Value>,
}

pub fn parse_message_create(d: &Value) -> Option<MessageCreateData> {
    let channel_id = d.get("channel_id")?.as_str()?.to_string();
    let guild_id = d.get("guild_id").and_then(|v| v.as_str()).map(String::from);
    let content = d.get("content")?.as_str()?.to_string();

    let author = d.get("author")?;
    let sender_id = author.get("id")?.as_str()?.to_string();
    let sender_name = author.get("global_name")
        .or_else(|| author.get("username"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let sender_is_bot = author.get("bot").and_then(|v| v.as_bool()).unwrap_or(false);

    let mentions: Vec<String> = d
        .get("mentions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|id| id.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let thread_id = d.get("thread")
        .and_then(|t| t.get("id"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let attachments = d.get("attachments")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    Some(MessageCreateData {
        channel_id,
        guild_id,
        thread_id,
        sender_id,
        sender_name,
        sender_is_bot,
        content,
        mentions,
        attachments,
    })
}

pub fn strip_mention(content: &str, bot_user_id: &str) -> String {
    content
        .replace(&format!("<@{}>", bot_user_id), "")
        .replace(&format!("<@!{}>", bot_user_id), "")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heartbeat_frame() {
        let frame = heartbeat_frame(Some(42));
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["op"], 1);
        assert_eq!(v["d"], 42);
    }

    #[test]
    fn test_heartbeat_frame_null_seq() {
        let frame = heartbeat_frame(None);
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["op"], 1);
        assert!(v["d"].is_null());
    }

    #[test]
    fn test_identify_frame() {
        let frame = identify_frame("my-token");
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["op"], 2);
        assert_eq!(v["d"]["token"], "my-token");
        assert_eq!(v["d"]["intents"], INTENTS);
    }

    #[test]
    fn test_parse_ready() {
        let d = serde_json::json!({
            "user": {"id": "12345", "username": "nexus-bot"}
        });
        let ready = parse_ready(&d).unwrap();
        assert_eq!(ready.bot_user_id, "12345");
    }

    #[test]
    fn test_parse_message_create_dm() {
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "hello",
            "author": {"id": "user1", "username": "bob"},
            "mentions": []
        });
        let msg = parse_message_create(&d).unwrap();
        assert_eq!(msg.channel_id, "ch1");
        assert_eq!(msg.sender_id, "user1");
        assert_eq!(msg.sender_name, "bob");
        assert!(!msg.sender_is_bot);
        assert!(msg.guild_id.is_none());
    }

    #[test]
    fn test_parse_message_create_global_name_preferred() {
        let d = serde_json::json!({
            "id": "msg3",
            "channel_id": "ch3",
            "content": "hi",
            "author": {"id": "user3", "username": "bob123", "global_name": "Bob"},
            "mentions": []
        });
        let msg = parse_message_create(&d).unwrap();
        assert_eq!(msg.sender_name, "Bob");
    }

    #[test]
    fn test_parse_message_create_guild_with_mention() {
        let d = serde_json::json!({
            "id": "msg2",
            "channel_id": "ch2",
            "guild_id": "guild1",
            "content": "<@botid> do something",
            "author": {"id": "user2", "username": "alice"},
            "mentions": [{"id": "botid", "username": "nexus-bot"}]
        });
        let msg = parse_message_create(&d).unwrap();
        assert_eq!(msg.guild_id, Some("guild1".to_string()));
        assert!(msg.mentions.contains(&"botid".to_string()));
    }

    #[test]
    fn test_strip_mention() {
        assert_eq!(strip_mention("<@123> hello", "123"), "hello");
        assert_eq!(strip_mention("<@!123> hello", "123"), "hello");
        assert_eq!(strip_mention("hello <@123> world", "123"), "hello  world");
        assert_eq!(strip_mention("no mention", "123"), "no mention");
    }
}
