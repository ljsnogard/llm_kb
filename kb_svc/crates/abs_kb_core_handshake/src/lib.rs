//! **系统层握手**：`kb_core` 公布自己 IPC 端点的那条通知。
//!
//! `kb_core` 以 `--handshake-prompt=stdio` 启动时，会在把端点准备好之后往
//! **stdout** 打一行 [`IpcReadyNotice`] 的 JSON，把"端点名字文件在哪"告诉启动它的
//! 父进程。启动方（`kb_core_starter`）读这一行，就知道该去哪找可连接的端点。
//!
//! # 它属于哪一层
//!
//! 客户端要跟 `kb_core` 谈成事，要过两道握手，它们的**消息都是公开协议**：
//!
//! | 层面 | 解决什么 | 消息定义在哪 |
//! | :--- | :--- | :--- |
//! | **系统层**（本 crate） | 找得到、连得上 | [`IpcReadyNotice`] + [`HandshakeNoticeKind`] |
//! | **应用层** | 谈得成 | `abs_kb_svc_v1_desktop` 的 `PROTOCOL_VERSION` / `ClientInfo` / `ServerInfo` / `Request::Hello` |
//!
//! 两者分开的理由是**依赖面**：系统层的消息只有一个"我准备好了、端点在名字文件里"，
//! 而应用层要带上工作区、会话、服务列表这些业务词汇。把系统层单独放一个 crate，
//! **只想启动并找到 `kb_core` 的调用方（`kb_core_starter`）就不必依赖整套业务协议**
//! （那会连带 `abs_llm` / `buffex` / `mm_ptr` 等一串依赖）。
//!
//! # 同一份定义，两个方向
//!
//! | 谁 | 做什么 |
//! | :--- | :--- |
//! | `kb_core`（`serve_`） | 用 [`IpcReadyNotice`] **序列化**，`writeln!` 到 stdout |
//! | `kb_core_starter` | `serde_json::from_str::<IpcReadyNotice>` **反序列化** |
//!
//! 两边共用这里的类型，所以改字段名/改 `event` 取值是**编译错误**，而不是
//! "跑起来才发现启动不了"——这正是把这条通知从"各写各的 JSON"提升为协议的目的。
//!
//! # 它不承诺什么
//!
//! 只承诺**文件名**：`kb_core` 会往那个名字文件里写"当前可连的端点名"，
//! 但那是等它真正开始 `accept` 时的事。拿到通知之后仍可能有短暂连不上，
//! 连接方必须带重试。
//!
//! # 示例
//!
//! ```
//! use abs_kb_core_handshake::{HandshakeNoticeKind, IpcReadyNotice};
//!
//! let line = r#"{"event":"ipc_ready","ipc_name_file":"/run/user/1000/llm_kb/kb-20260918-abc.ipc","protocol_version":1,"pid":1234}"#;
//! let notice: IpcReadyNotice = serde_json::from_str(line).expect("应当能解开 kb_core 打的那一行");
//!
//! assert_eq!(notice.event, HandshakeNoticeKind::IpcReady);
//! assert_eq!(notice.ipc_name_file, "/run/user/1000/llm_kb/kb-20260918-abc.ipc");
//! ```

mod system_;

pub use system_::{HandshakeNoticeKind, IpcReadyNotice};
