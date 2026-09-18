//! 协议版本、双方身份，以及服务端的整体状态；以及**两个层面的握手**。
//!
//! # 两个层面的握手，作用不同
//!
//! 客户端要跟 `kb_core` 谈成事，必须先过两道握手。把它们分开写清楚，是为了让
//! "找得到"与"谈得成"互不污染：传输怎么换，业务协议都不动。
//!
//! **两个层面的消息都是本 crate 定义的公开协议**——这正是"两个进程不许各自
//! 造一份字段名"的落点。区别只在**消息之外的机制**归谁：挑哪种内核端点、
//! 端点放哪、失败怎么重试，属于传输实现。
//!
//! ```text
//! ① 系统层（底层）握手 —— 「找得到、连得上」
//!    客户端进程 ──► kb_core 进程
//!    解决：内核端点在哪、连接怎么建立。
//!    消息：IpcReadyNotice（见下）——kb_core 用 --handshake-prompt=stdio 时
//!          往 stdout 打的那一行 JSON，把 IPC 端点名字文件告诉启动它的父进程。
//!    机制（不进协议，由传输实现决定）：
//!      · kb_svc_servo_ipc：读运行时目录里的 kb-<日期>-<uuid>.ipc，拿里面的端点名连接；
//!      · kb_core_starter：起 kb_core 子进程、读那一行 IpcReadyNotice 并解析；
//!      · 将来别的传输（socket / 共享内存 / …）各自决定怎么把消息送到。
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
//!   把这些机制留在实现 crate 里，协议就不必跟着改。但**双方在系统层交换的
//!   那条消息**是一条跨进程约定，所以它的类型定义在这里——否则 `kb_core` 与
//!   它的启动方会各写一份字段名，改名只能靠跑起来才发现。
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

// ── 系统层握手：kb_core 公布 IPC 端点的那条通知 ─────────────────────────
//
// 与应用层握手（上面那几个类型 + `Request::Hello`）是**两个层面**，但都属于
// 协议 v1：`kb_core` 与它的启动方（`kb_core_starter`）必须用同一份定义，
// 否则改个字段名只能靠跑起来才发现。

/// [`IpcReadyNotice::event`] 的取值：端点已就绪。
///
/// 单独列成一个类型而不是裸 `String`，是为了让"种类"这件事在编译期就有约束，
/// 也为将来可能出现的其它系统层通知留位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandshakeNoticeKind {
    /// `kb_core` 已经把 IPC 端点名字文件准备好，父进程可以照着它去连接了。
    #[serde(rename = "ipc_ready")]
    IpcReady,
}

/// **系统层握手**：`kb_core` 就绪后公布 IPC 端点名字文件的那条通知。
///
/// `kb_core` 以 `--handshake-prompt=stdio` 启动时，会在 `Listener::bind` 之后往
/// **stdout** 打一行该类型的 JSON（打完就 flush；stdout 其余时间保持干净）：
///
/// ```json
/// {"event":"ipc_ready","ipc_name_file":"…/kb-20260918-….ipc","protocol_version":1,"pid":1234}
/// ```
///
/// # 它承诺什么、不承诺什么
///
/// 只承诺**文件名**：`kb_core` 会往那个名字文件里写"当前可连的端点名"，但那是
/// 等它真正开始 `accept` 时的事。所以拿到通知之后**仍可能有短暂连不上**，
/// 连接方必须带重试（`kb_svc_servo_ipc::Client` 就是这么做的）。
///
/// # 谁发、谁收
///
/// - 发：`kb_core`（`--handshake-prompt=stdio`）；
/// - 收：启动它的父进程——`kb_core_starter`（`kb_core_rproxy` 与桌面客户端都用它）。
///
/// 独立运行、不需要父进程转告端点的客户端（直接扫运行时目录的那种）不经过这条通知。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpcReadyNotice {
    /// 通知种类。
    pub event: HandshakeNoticeKind,

    /// IPC 端点名字文件的路径。
    ///
    /// 语义是"这个 `kb_core` 实例"，**不是**"现在就能连上"：文件内容才是端点名。
    pub ipc_name_file: String,

    /// `kb_core` 使用的协议版本（[`PROTOCOL_VERSION`]）。
    ///
    /// 它只是报备：真正"版本不一致就拒绝"发生在应用层握手（`Request::Hello`）。
    pub protocol_version: u32,

    /// `kb_core` 的进程号（日志与排错用）。
    pub pid: u32,
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

    /// 测试系统层就绪通知的 JSON 形状与 `kb_core` 实际打的那一行一致。
    ///
    /// - 手段：构造一条 `IpcReadyNotice` 并序列化；再把 `kb_core` 会打出的字面
    ///   JSON 反序列化回来。
    /// - 判断：序列化结果里 `event` 是字符串 `"ipc_ready"`（不是枚举名
    ///   `"IpcReady"`），四个字段名与线格式一致；字面量能解出同样的字段值。
    ///
    ///   这条守的是"通知类型属于公开协议"：字段名与 `event` 取值一旦改动，
    ///   启动方（`kb_core_starter`）会立刻解不开，而不是悄悄错位。
    #[test]
    fn ipc_ready_notice_matches_the_stdio_wire_format_() {
        let notice = IpcReadyNotice {
            event: HandshakeNoticeKind::IpcReady,
            ipc_name_file: "/run/user/1000/llm_kb/kb-20260918-abc.ipc".to_string(),
            protocol_version: PROTOCOL_VERSION,
            pid: 1234,
        };

        let json = serde_json::to_string(&notice).expect("应当能序列化");
        assert!(json.contains(r#""event":"ipc_ready""#), "实际 JSON: {json}");
        assert!(
            json.contains(r#""ipc_name_file":"/run/user/1000/llm_kb/kb-20260918-abc.ipc""#),
            "实际 JSON: {json}"
        );
        assert!(
            json.contains(r#""protocol_version":1"#),
            "实际 JSON: {json}"
        );
        assert!(json.contains(r#""pid":1234"#), "实际 JSON: {json}");

        let literal = r#"{"event":"ipc_ready","ipc_name_file":"/run/user/1000/llm_kb/kb-20260918-abc.ipc","protocol_version":1,"pid":1234}"#;
        let parsed: IpcReadyNotice =
            serde_json::from_str(literal).expect("kb_core 打出的那一行应当能解开");
        assert_eq!(parsed, notice);
    }

    /// 测试系统层通知在 **postcard** 上也能往返，且不认识的 `event` 会被拒绝。
    ///
    /// - 手段：把 `IpcReadyNotice` 用 postcard 编解码一次；再分别喂进
    ///   `"event":"IpcReady"`（枚举名而不是线上取值）与 `"event":"nope"`。
    /// - 判断：postcard 往返相等；两种坏 `event` 都解不开——`#[serde(rename)]`
    ///   只认线上那个字符串，拼错不会"尽力而为"地放过去。
    #[test]
    fn ipc_ready_notice_round_trips_through_postcard_() {
        let notice = IpcReadyNotice {
            event: HandshakeNoticeKind::IpcReady,
            ipc_name_file: "/tmp/kb/run/kb-20260918-abc.ipc".to_string(),
            protocol_version: PROTOCOL_VERSION,
            pid: 7,
        };

        let bytes = postcard::to_allocvec(&notice).expect("postcard 应当能编码");
        let parsed: IpcReadyNotice = postcard::from_bytes(&bytes).expect("postcard 应当能解码");
        assert_eq!(parsed, notice);

        for bad in [
            r#"{"event":"IpcReady","ipc_name_file":"/tmp/x.ipc","protocol_version":1,"pid":1}"#,
            r#"{"event":"nope","ipc_name_file":"/tmp/x.ipc","protocol_version":1,"pid":1}"#,
        ] {
            assert!(
                serde_json::from_str::<IpcReadyNotice>(bad).is_err(),
                "不该解开: {bad}"
            );
        }
    }
}
