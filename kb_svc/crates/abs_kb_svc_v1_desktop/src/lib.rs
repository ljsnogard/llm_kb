//! 桌面客户端（`kb_admin_desktop`）与 `kb_core` 之间交换的数据（协议 v1）。
//!
//! # 这个模块回答什么
//!
//! 它是"桌面客户端要和 `kb_core` 交流哪些数据"的**唯一定义处**。客户端侧
//! （Dart / flutter_rust_bridge）与 `kb_core` 侧都应当以这里的类型为准，
//! 不允许各自维护一份镜像定义。
//!
//! # 三个方向
//!
//! ```text
//!   桌面客户端 ──Request──►  kb_core     （提问、取消、增删改查配置与工作区）
//!   桌面客户端 ◄──Reply────  kb_core     （与 Request 一一对应的结果）
//!   桌面客户端 ◄──Event────  kb_core     （流式增量、状态变化等主动推送）
//! ```
//!
//! `Reply` 与 `Request` 通过 [`RequestEnvelope::request_id`] 关联；
//! `Event` 不与任何请求关联，它是服务端主动推送的。
//!
//! # 文件划分
//!
//! 本目录按**数据概念**拆文件，而不是按"类型/函数"分层；
//! 只有当若干类型之间存在**内部共享逻辑或私有访问**时才放在同一个文件里
//! （目前只有 `ids_` 因为共用一个私有宏而聚合）。
//!
//! | 文件 | 内容 |
//! | :--- | :--- |
//! | `ids_` | 标识类型与它们的生成规则 |
//! | `handshake_` | 两个层面的握手：协议版本、双方身份、服务端整体状态（应用层）+ 就绪通知（系统层） |
//! | `content_` | 一条消息及其组成部分（增量、工具调用、用量、提示） |
//! | `service_` | LLM 服务配置与 API key 的更新语义 |
//! | `workspace_` | 工作区与会话（kb_core 管理、多端同步） |
//! | `fs_` | 工作区目录浏览 |
//! | `error_` | 业务错误 |
//! | `request_` / `reply_` / `event_` | 三个方向上的消息 |
//! | `envelope_` | 请求/应答的关联信封 |
//! | `rpc_` | **按业务域拆分的异步 RPC trait**（`TrWorkspaceService` 等） |
//!
//! 导出由本文件统一控制：子模块是私有的，`pub use` 决定对外可见的名字，
//! 因此**公开路径始终是 `abs_kb_svc_v1_desktop::<类型名>`**，
//! 内部的挪动不会影响使用者。
//!
//! # 两条贯穿全局的约定
//!
//! ## 1. 表示形式必须是"非自描述编解码器"也能处理的
//!
//! 实现候选 `kb_svc_servo_ipc` 用 ipc-channel，而 ipc-channel 0.23 的内部编解码器是
//! **postcard**——它**不自描述**。实测踩到的两个坑：
//!
//! | 写法 | JSON | postcard |
//! | :--- | :--- | :--- |
//! | 内部标签 `#[serde(tag = "type")]` | ✅ `{"type":"ask",…}` | ❌ **编码成功但解码报错**（"This is a feature that PostCard will never implement"） |
//! | `#[serde(skip_serializing_if = "…")]` | ✅ 字段被省略 | ❌ **解码报错**（`DeserializeUnexpectedEnd`） |
//! | 外部标签（serde 默认）+ 不用 `skip_serializing_if` | ✅ | ✅ |
//!
//! 因此本模块有两条硬性写法要求：
//!
//! 1. **枚举一律使用 serde 默认的外部标签**（`{"Ask": {…}}`）；
//! 2. **不使用 `skip_serializing_if`**：可选字段一律编码为 `Option::None` 分支，
//!    空集合编码为 `[]`。`#[serde(default)]` 保留，供 JSON 等自描述格式容错。
//!
//! 同理**不要使用** `#[serde(flatten)]`——它同样只对自描述格式有意义。
//!
//! 代价是 JSON 调试输出比 `"type":"ask"` 啰嗦；收益是它真的能在目标传输上跑起来。
//! `envelope_.rs` 里有一组用 postcard 做**真解码**的往返测试来守住这两条约定。
//!
//! ## 2. 只发送差异，不发送整棵树
//!
//! 工作区与会话是扁平列表 + 按需拉取，客户端自己组装
//! "工作区 → 会话 → 消息"的界面结构。
//!
//! # 其它约定
//!
//! - 所有标识都是新类型（[`WorkspaceId`] / [`SessionId`] / [`TurnId`] /
//!   [`ServiceId`] / [`RequestId`] / [`LocalId`]），避免把会话标识传给需要回合标识的位置；
//! - **API key 永不明文回传**：服务端只回掩码（见 [`MASKED_API_KEY`] 与 [`ApiKeyUpdate`]）；
//! - 时间统一用"自 Unix 纪元起的毫秒数"（`i64`），不传格式化字符串。
//!
//! # 示例
//!
//! ```
//! use abs_kb_svc_v1_desktop::{AskRequest, Request, RequestEnvelope, ServiceId, TurnId};
//!
//! // 一次提问
//! let request = Request::Ask(AskRequest {
//!     workspace_id: "w-1".into(),
//!     session_id: "s-1".into(),
//!     turn_id: TurnId::new("t-1"),
//!     question: "你好".to_string(),
//!     service_id: Some(ServiceId::new("deepseek")),
//! });
//!
//! let envelope = RequestEnvelope::new("q-1", request);
//! let json = serde_json::to_string(&envelope).expect("应当能序列化");
//! assert!(json.contains(r#""Ask""#), "实际 JSON: {json}");
//!
//! let parsed: RequestEnvelope = serde_json::from_str(&json).expect("应当能反序列化");
//! assert_eq!(parsed, envelope);
//! ```

// 子模块一律不公开导出，因此模块名以 `_` 结尾（`AGENTS.md` 第 5 条）。
// 下面的 `pub use` 逐个列举要导出的名字，不使用通配符——这样"对外暴露了什么"
// 在本文件里一眼可见，新增类型不会因为一个 `*` 就悄悄进入公开 API。
mod content_;
mod envelope_;
mod error_;
mod event_;
mod fs_;
mod handshake_;
mod ids_;
mod reply_;
mod request_;
mod rpc_;
mod service_;
mod workspace_;

pub use content_::{Notice, TokenUsage, ToolCallRecord, Turn, TurnState};
pub use envelope_::{ReplyEnvelope, RequestEnvelope};
pub use error_::{ErrorCode, ErrorReply};
pub use event_::{
    ErrorEvent, Event, SessionChanged, TextDelta, ToolCallEvent, TurnFinished, TurnStarted,
    UsageEvent,
};
pub use fs_::{DirEntry, DirEntryKind, DirectoryListing};
pub use handshake_::{
    ClientInfo, HandshakeNoticeKind, IpcReadyNotice, PROTOCOL_VERSION, ServerInfo, ServerState,
};
pub use ids_::{LocalId, RequestId, ServiceId, SessionId, TurnId, WorkspaceId};
pub use reply_::Reply;
pub use request_::{
    AddWorkspaceRequest, AskRequest, CreateSessionRequest, Request, UpsertServiceRequest,
};
pub use rpc_::{
    RpcError, TrGeneration, TrHandshake, TrKbEndpoint, TrKbService, TrSessionService,
    TrWorkspaceService,
};
pub use service_::{ApiKeyUpdate, MASKED_API_KEY, ServiceList, ServiceSummary};
pub use workspace_::{SessionDetail, SessionList, SessionSummary, Workspace, WorkspaceList};
