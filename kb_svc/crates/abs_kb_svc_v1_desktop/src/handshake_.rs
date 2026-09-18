//! 协议版本、双方身份，以及服务端的整体状态；以及**两个层面的握手**。
//!
//! # 两个层面的握手，作用不同
//!
//! 客户端要跟 `kb_core` 谈成事，必须先过两道握手。把它们分开写清楚，是为了让
//! "找得到"与"谈得成"互不污染：传输怎么换，业务协议都不动。
//!
//! **两个层面的消息都是公开协议**——这正是"两个进程不许各自造一份字段名"的落点。
//! 区别有两处：**消息定义在哪个 crate**（见下表），以及**消息之外的机制**归谁
//! （挑哪种内核端点、端点放哪、失败怎么重试，属于传输实现）。
//!
//! ```text
//! ① 系统层（底层）握手 —— 「找得到、连得上」
//!    客户端进程 ──► kb_core 进程
//!    解决：内核端点在哪、连接怎么建立。
//!    消息：IpcReadyNotice —— 定义在 **abs_kb_core_handshake**（见下），
//!          本模块把它转出来，所以 `abs_kb_svc_v1_desktop::IpcReadyNotice` 也能用。
//!          kb_core 用 --handshake-prompt=stdio 时往 stdout 打那一行 JSON，
//!          把 IPC 端点名字文件告诉启动它的父进程。
//!    机制（不进协议，由传输实现决定）：
//!      · kb_svc_servo_ipc：读运行时目录里的 kb-<日期>-<uuid>.ipc，拿里面的端点名连接；
//!      · kb_core_starter：起 kb_core 子进程、读那一行 IpcReadyNotice 并解析；
//!      · 将来别的传输（socket / 共享内存 / …）各自决定怎么把消息送到。
//!
//! ② 应用层握手 —— 「谈得成」
//!    客户端 ──[Request::Hello]──► kb_core
//!    客户端 ◄─[Reply::Hello]───  kb_core     （身份 + 协议版本）
//!    客户端 ◄─[Event::Ready]───  kb_core     （服务端状态：能不能干活、有哪些服务）
//!    消息定义在**本 crate**（本模块与 request_ / reply_ / event_）。
//! ```
//!
//! 为什么系统层的消息单独一个 crate：它只有"我准备好了、端点在名字文件里"，
//! 不需要工作区 / 会话 / 服务这些业务词汇。`kb_core_starter` 那种**只想启动并
//! 找到 `kb_core`** 的调用方因此不必依赖整套业务协议（以及它背后的
//! `abs_llm` / `buffex` / `mm_ptr`）。理由与依赖方向见
//! `abs_kb_core_handshake` 的 crate 文档。
//!
//! 为什么要分两层：
//!
//! - **系统层只关心"怎么把字节送到"**。`kb_core` 必然长期需要本机 IPC，
//!   而具体手段（Unix domain socket / ipc-channel / 命名管道 / 代理）会变；
//!   把这些机制留在实现 crate 里，协议就不必跟着改。但**双方在系统层交换的
//!   那条消息**是一条跨进程约定，所以它有类型定义（在 `abs_kb_core_handshake`）
//!   ——否则 `kb_core` 与它的启动方会各写一份字段名，改名只能靠跑起来才发现。
//! - **应用层只关心"对面是谁、能不能干活"**。它必须在**业务请求之前**完成，
//!   而且与传输无关——无论客户端是直连本机 IPC，还是经 `kb_core_rproxy` 从
//!   局域网过来，这一段都必须原样发生。
//!
//! 因此：**系统层握手成功不代表业务可用**；应用层握手才是"可以开始发业务请求"
//! 的分界线。
//!
//! # 应用层握手的三个消息，携带的信息刻意不同
//!
//! - [`Request::Hello`](crate::Request::Hello)：客户端自报身份与版本；
//! - [`Reply::Hello`](crate::Reply::Hello)：服务端只回**身份与版本**
//!   （要不要继续谈下去）；
//! - [`Event::Ready`](crate::Event::Ready)：服务端随后推**当前状态**
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
/// 既用于握手后的第一条推送（[`Event::Ready`](crate::Event::Ready)），
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

// ── 系统层握手：类型在独立 crate 里，这里只把它转出来 ──────────────────
//
// 那条通知只有一个"我准备好了、端点在名字文件里"，不需要工作区 / 会话 / 服务
// 这些业务词汇；把它放在 `abs_kb_core_handshake` 里，`kb_core_starter` 那种
// **只想启动并找到 kb_core** 的调用方就不必依赖整套业务协议。
//
// 这里转出来，是为了让桌面协议的公开面同时看得到两个层次（桌客户端本来就要先
// 起 / 找本机的 kb_core），也让 `abs_kb_svc::v1::desktop::IpcReadyNotice` 这个
// 既有路径保持不变。
pub use abs_kb_core_handshake::{HandshakeNoticeKind, IpcReadyNotice};

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
