//! 协议版本、双方身份，以及服务端的整体状态。
//!
//! 握手是**必须**的第一步：客户端发 [`Request::Hello`](crate::v1::desktop::Request::Hello)，
//! 服务端回 [`Reply::Hello`](crate::v1::desktop::Reply::Hello)，随后推一条
//! [`Event::Ready`](crate::v1::desktop::Event::Ready)。
//!
//! 三者携带的信息刻意不同：`Hello` 只交换**身份与版本**（要不要继续谈下去），
//! `Ready` 携带**当前状态**（能不能提问、有哪些服务）。这样版本不一致时
//! 可以在拿到状态之前就拒绝连接。

use serde::{Deserialize, Serialize};

use super::service_::ServiceSummary;
use super::ids_::ServiceId;

/// 协议版本。
///
/// 握手时双方交换该值；不一致时应当明确拒绝连接，而不是"尽力而为"地继续。
pub const PROTOCOL_VERSION: u32 = 1;

/// 客户端自报的身份。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientInfo {
    /// 客户端名字，例如 `kb_admin_desktop`。
    pub client_name: String,

    /// 客户端版本。
    pub client_version: String,

    /// 客户端支持的协议版本（[`PROTOCOL_VERSION`]）。
    pub protocol_version: u32,
}

/// 服务端自报的身份。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    /// 服务端版本。
    pub server_version: String,

    /// 服务端使用的协议版本（[`PROTOCOL_VERSION`]）。
    pub protocol_version: u32,
}

/// 服务端的整体状态。
///
/// 既用于握手后的第一条推送（[`Event::Ready`](crate::v1::desktop::Event::Ready)），
/// 也用于"插件上下线 / 服务配置变化"这类状态刷新。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerState {
    /// 当前是否有 LLM 插件在线。
    ///
    /// 为 `false` 时客户端应当禁用提问入口，而不是让用户发出必然失败的问题。
    pub plugin_online: bool,

    /// 服务端版本。
    pub server_version: String,

    /// 已配置的 LLM 服务。
    pub services: Vec<ServiceSummary>,

    /// 当前生效的服务。
    #[serde(default)]
    pub active_service: Option<ServiceId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试握手信息往返序列化，且版本号被保留。
    ///
    /// - 手段：构造 `ClientInfo` 与 `ServerInfo`，序列化后反序列化。
    /// - 判断：往返结果与原值相等，且 `protocol_version` 与 [`PROTOCOL_VERSION`] 一致——
    ///   这直接支撑"版本不一致就拒绝"这条约定。
    #[test]
    fn handshake_round_trips_() {
        let client = ClientInfo {
            client_name: "kb_admin_desktop".to_string(),
            client_version: "0.1.0".to_string(),
            protocol_version: PROTOCOL_VERSION,
        };
        let json = serde_json::to_string(&client).expect("应当能序列化");
        let parsed: ClientInfo = serde_json::from_str(&json).expect("应当能反序列化");
        assert_eq!(parsed, client);
        assert_eq!(parsed.protocol_version, PROTOCOL_VERSION);

        let server = ServerInfo {
            server_version: "0.1.0".to_string(),
            protocol_version: PROTOCOL_VERSION,
        };
        let json = serde_json::to_string(&server).expect("应当能序列化");
        let parsed: ServerInfo = serde_json::from_str(&json).expect("应当能反序列化");
        assert_eq!(parsed, server);
    }

    /// 测试可选字段在编码时**始终出现**（为 postcard 兼容）。
    ///
    /// - 手段：构造 `ServerState::default()` 并序列化。
    /// - 判断：JSON 中出现 `"active_service":null`。这是**刻意**的：
    ///   postcard 不自描述，若用 `skip_serializing_if` 省略字段，解码方会因为
    ///   缺少那一个字节而失败（实测报 `DeserializeUnexpectedEnd`）。
    ///
    ///   因此本模块**禁止**使用 `skip_serializing_if`，可选字段一律编码为
    ///   `Option` 的 `None` 分支；`#[serde(default)]` 保留，供 JSON 等
    ///   自描述格式容错。
    #[test]
    fn optional_fields_are_always_encoded_() {
        let state = ServerState::default();
        let json = serde_json::to_string(&state).expect("应当能序列化");
        assert!(
            json.contains(r#""active_service":null"#),
            "实际 JSON: {json}"
        );

        let parsed: ServerState = serde_json::from_str(&json).expect("应当能反序列化");
        assert!(!parsed.plugin_online);
    }
}
