//! 系统层握手通知的类型：`kb_core` 就绪后公布 IPC 端点名字文件的那条消息。

use serde::{Deserialize, Serialize};

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

/// 系统层握手：`kb_core` 就绪后公布 IPC 端点名字文件的那条通知。
///
/// `kb_core` 以 `--handshake-prompt=stdio` 启动时，会在 `Listener::bind` 之后往
/// **stdout** 打一行该类型的 JSON（打完就 flush；stdout 其余时间保持干净）：
///
/// ```json
/// {"event":"ipc_ready","ipc_name_file":"…/kb-20260918-….ipc","protocol_version":1,"pid":1234}
/// ```
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

    /// `kb_core` 使用的协议版本。
    ///
    /// 它只是报备：真正"版本不一致就拒绝"发生在应用层握手（`Request::Hello`，
    /// 见 `abs_kb_svc_v1_desktop`）。
    pub protocol_version: u32,

    /// `kb_core` 的进程号（日志与排错用）。
    pub pid: u32,
}

#[cfg(test)]
mod tests_ {
    use super::*;

    /// 测试通知的 JSON 形状与 `kb_core` 实际打的那一行一致。
    ///
    /// - 手段：构造一条 [`IpcReadyNotice`] 并序列化；再把 `kb_core` 会打出的字面
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
            protocol_version: 1,
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

    /// 测试通知在 **postcard** 上也能往返，且不认识的 `event` 会被拒绝。
    ///
    /// - 手段：把 [`IpcReadyNotice`] 用 postcard 编解码一次；再分别喂进
    ///   `"event":"IpcReady"`（枚举名而不是线上取值）与 `"event":"nope"`。
    /// - 判断：postcard 往返相等；两种坏 `event` 都解不开——`#[serde(rename)]`
    ///   只认线上那个字符串，拼错不会"尽力而为"地放过去。
    ///
    ///   留 postcard 这一条，是因为本仓库的协议类型的通用约束是"不能依赖自描述
    ///   格式"（见 `abs_kb_svc_v1_desktop` 的模块文档）；虽然这条通知只走 JSON，
    ///   让它同时满足那条约束不需要任何额外写法。
    #[test]
    fn ipc_ready_notice_round_trips_through_postcard_() {
        let notice = IpcReadyNotice {
            event: HandshakeNoticeKind::IpcReady,
            ipc_name_file: "/tmp/kb/run/kb-20260918-abc.ipc".to_string(),
            protocol_version: 1,
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
