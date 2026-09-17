//! 协议版本、双方身份，以及服务端的整体状态；以及**两个层面的握手**。
//!
//! # 两个层面的握手，作用不同
//!
//! 客户端要跟 `kb_core` 谈成事，必须先过两道握手。把它们分开写清楚，是为了让
//! "找得到"与"谈得成"互不污染：传输怎么换，业务协议都不动。
//!
//! ```text
//! ① 系统层（底层）握手 —— 「找得到、连得上」
//!    客户端进程 ──► kb_core 进程
//!    解决：内核端点在哪、连接怎么建立。
//!    格式由**传输实现**决定，不进本 crate：
//!      · kb_svc_servo_ipc：读运行时目录里的 kb-<日期>-<uuid>.ipc，拿里面的端点名连接；
//!      · kb_core_rproxy：启动 kb_core 时用 --handshake-prompt=stdio 读那一行通知；
//!      · 将来别的传输（socket / 共享内存 / …）各定各的。
//!
//! ② 应用层握手 —— 「谈得成」
//!    客户端 ──[Request::Hello]──► kb_core
//!    客户端 ◄─[Reply::Hello]───  kb_core     （身份 + 协议版本）
//!    客户端 ◄─[Event::Ready]───  kb_core     （服务端状态：能不能干活、有哪些服务）
//!    格式就是本模块与 request_ / reply_ / event_ 里定义的类型，**属于协议 v1**。
//! ```
//!
//! 为什么要分两层：
//!
//! - **系统层只关心"怎么把字节送到"**。`kb_core` 必然长期需要本机 IPC，
//!   而具体手段（Unix domain socket / ipc-channel / 命名管道 / 代理）会变；
//!   把这些细节留在实现 crate 里，协议就不必跟着改。
//! - **应用层只关心"对面是谁、能不能干活"**。它必须在**业务请求之前**完成，
//!   而且与传输无关——无论客户端是直连本机 IPC，还是经 `kb_core_rproxy` 从
//!   局域网过来，这一段都必须原样发生。
//!
//! 因此：**系统层握手成功不代表业务可用**；应用层握手才是"可以开始发业务请求"
//! 的分界线。
//!
//! # 应用层握手的三个消息，携带的信息刻意不同
//!
//! - [`Request::Hello`](crate::v1::desktop::Request::Hello)：客户端自报身份与版本；
//! - [`Reply::Hello`](crate::v1::desktop::Reply::Hello)：服务端只回**身份与版本**
//!   （要不要继续谈下去）；
//! - [`Event::Ready`](crate::v1::desktop::Event::Ready)：服务端随后推**当前状态**
//!   （能不能提问、有哪些服务）。
//!
//! 这样版本不一致时，可以在拿到状态之前就明确拒绝连接，而不是"尽力而为"地继续。

use serde::{Deserialize, Serialize};

use super::ids_::ServiceId;
use super::service_::ServiceSummary;

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
