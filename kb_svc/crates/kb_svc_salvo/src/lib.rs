//! # kb_svc_salvo
//!
//! 知识库服务的 Salvo 实现。
//!
//! 本 crate **只是库**，不提供二进制目标；进程的启动与编排由 `kb_core` 负责。
//!
//! 现阶段（PoC 阶段）crate 内只包含：
//!
//! - [`plugin_socket`]：插件通道 UDS 路径的生成与生命周期管理（已定稿，属正式内容）；
//! - [`poc`]：验证「Unix domain socket 上能否跑 WebSocket」所需的临时代码。

pub mod error;
pub mod plugin_socket;
pub mod poc;
