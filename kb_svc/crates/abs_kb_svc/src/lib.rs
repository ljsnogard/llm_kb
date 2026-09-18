//! # abs_kb_svc
//!
//! 知识库服务协议的**聚合 crate**：把各个协议 crate 摆到一个稳定的路径下，
//! 让调用方不必逐个记住"哪一版、哪一类对端的协议在哪个 crate 里"。
//!
//! # 聚合了什么
//!
//! | 路径 | 内容 | 定义在 |
//! | :--- | :--- | :--- |
//! | [`v1::desktop`] | 桌面客户端 `kb_admin_desktop` 与 `kb_core` 交换的数据，以及按业务域拆分的异步 RPC trait | `abs_kb_svc_v1_desktop` |
//! | [`v1::desktop::IpcReadyNotice`] / [`v1::desktop::HandshakeNoticeKind`] | **系统层握手**：`kb_core` 公布 IPC 端点的那条通知（两个层次里的底层那一个） | `abs_kb_core_handshake`，由上一个转出 |
//! | [`v1::desktop::ClientInfo`] / [`v1::desktop::ServerInfo`] / [`v1::desktop::PROTOCOL_VERSION`] | **应用层握手**：双方身份与协议版本 | `abs_kb_svc_v1_desktop` |
//!
//! 将来增加"别的对端"（例如 `v1::plugin`）或"别的版本"时，也在这里加一行，
//! 调用方的路径不变。
//!
//! # 为什么保留它
//!
//! 协议按"版本 + 对端"拆成独立 crate 之后，实现方（`kb_core`、`kb_svc_servo_ipc`、
//! `kb_core_rproxy`）仍然只写 `abs_kb_svc::v1::desktop::X`。**换布局不动调用方**，
//! 这就是它作为聚合层存在的全部理由——它自己的 `[dependencies]` 里只有协议 crate，
//! 没有任何运行时、传输或业务实现。
//!
//! # 什么时候不必经过它
//!
//! 需要**整套业务协议**的调用方才该依赖本 crate。只做一件事的调用方直接依赖对应
//! 的细粒度 crate 更划算，例如：
//!
//! - 只想启动并找到 `kb_core` 的 `kb_core_starter` → 直接依赖 `abs_kb_core_handshake`，
//!   不会因此拖进 `abs_llm` 等一串业务依赖。
//!
//! # 它不是什么
//!
//! 不依赖任何具体传输（ipc-channel / socket / 同进程线程），也不定义"数据在线上
//! 长什么样"——那是实现 crate（`kb_svc_servo_ipc` 等）的职责。协议本身的设计约定
//! 见 `abs_kb_svc_v1_desktop` 的 crate 文档与 `README.md`。
//!
//! # 示例
//!
//! 聚合出来的路径与拆分前完全一致：
//!
//! ```
//! use abs_kb_svc::v1::desktop::{Request, RequestEnvelope, WorkspaceId};
//!
//! let envelope = RequestEnvelope::new(
//!     "q-1",
//!     Request::ListSessions {
//!         workspace_id: WorkspaceId::new("w-1"),
//!     },
//! );
//! assert_eq!(envelope.request_id.to_string(), "q-1");
//! ```

/// 协议 v1。
///
/// 版本号在 [`desktop::PROTOCOL_VERSION`](abs_kb_svc_v1_desktop::PROTOCOL_VERSION)
/// 中给出。握手时双方交换该值，不一致时应当明确拒绝，而不是"尽力而为"地继续。
///
/// 这里按"对端"分类：目前只有桌面客户端 `kb_admin_desktop`（[`desktop`](abs_kb_svc_v1_desktop)）。
/// 插件（`kb_plugins/*`）与 `kb_core` 的通信内容是另一组消息，将来以
/// `v1::plugin` 之类的名字加进来；两者**不共用**消息类型，因为语义完全不同
/// （客户端关心交互，插件关心 provider 调用）。
pub mod v1 {
    /// 桌面客户端 `kb_admin_desktop` 与 `kb_core` 交换的数据。
    ///
    /// 别名到 `abs_kb_svc_v1_desktop`：路径 `abs_kb_svc::v1::desktop::<类型名>`
    /// 与拆分前完全一致。按 `AGENTS.md` 第 5 条，这里不用通配符——直接**别名整个
    /// crate**，这样"这一版这一端的协议有哪些东西"仍然只在新 crate 的根文件里
    /// 逐项列举一次，不会出现两份清单。
    pub use abs_kb_svc_v1_desktop as desktop;
}
