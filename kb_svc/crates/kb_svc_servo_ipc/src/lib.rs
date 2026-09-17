//! # kb_svc_servo_ipc
//!
//! 知识库服务的**进程间通信实现**：把 [`abs_kb_svc`] 定义的通信内容与按域
//! 异步 RPC trait 落到 **servo/ipc-channel** 上。
//!
//! 选型理由、实测数据与待决策项见
//! `dev-notes/kb_svc_servo_ipc-20260917-1548.md`；本文件只讲形状。
//!
//! # 两端各是什么
//!
//! ```text
//!   kb_core（服务端）                          kb_admin_desktop（客户端）
//!   ───────────────                           ──────────────────────
//!   Listener::bind(runtime_dir)                Client::connect(runtime_dir)
//!        │  accept()（阻塞）                        │  读名字文件 + 重试
//!        ▼                                          ▼
//!   Connection ── serve(&service) ──┐          Client: impl TrWorkspaceService
//!        │                          │                  + TrSessionService
//!        │  三通道：请求↑ / 应答↓ / 事件↓                （trait 方法 → 请求 → 等应答）
//!        └──────────────────────────┴──────────────────┘
//! ```
//!
//! # 一条连接上的三条通道
//!
//! 引导消息（客户端连上之后发的第一条消息）是一个端点三元组：
//!
//! ```text
//! (IpcReceiver<RequestEnvelope>,   // 上行：请求
//!  IpcSender<ReplyEnvelope>,       // 下行：应答，用 request_id 配回
//!  IpcSender<Event>)               // 下行：服务端主动推送
//! ```
//!
//! 三条通道各自独立：请求可以在服务端并发处理、应答乱序回来也不会串
//! （`RequestEnvelope::request_id` 兜底）；事件不会被大应答堵住。
//! 注意 ipc-channel 的通道**无界且 `send` 不阻塞**，所以背压必须在语义层自己做
//! （见 `abs_kb_svc` README §5 第 6 条）。
//!
//! # 引导（rendezvous）与多客户端
//!
//! ipc-channel 的 `IpcOneShotServer` **只能接受一次连接**。做法是：
//!
//! ```text
//! kb_core 侧（accept 是阻塞调用，必须从阻塞线程调用）:
//!   启动时: 生成 <runtime-dir>/kb-<日期>-<uuid>.ipc，并清掉上次残留的名字文件
//!   循环  { 新建 one-shot 端点 → 把端点名原子写进名字文件 → accept → 处理 }
//!          接受之后立刻把名字文件内容清空（那个端点已经用掉了）
//! 客户端侧:
//!   循环  { 在运行时目录里找内容非空的 kb-*.ipc → connect；失败就重试 }
//! ```
//!
//! **文件名由 `kb_core` 决定**，沿用 `kb_svc_salvo` 时期的「日期 + UUID」约定
//! （见 [`new_name_file_in`]）：每次启动都独一无二，历史残留不会挡路。
//! 这条约定与传输库无关，换掉 ipc-channel 也照样成立。
//! 文件名的语义是"服务端实例"，内容才是"当前可连的端点名"。
//!
//! 实测：3 个客户端并发抢同一个端点时，只有一个能连上，其余快速失败后重试成功。
//!
//! > `ipc-channel` 自己的 socket 放在系统临时目录里（`IpcOneShotServer` 没有
//! > 提供指定路径的接口），所以"操作系统层面的 socket 路径"仍由它决定；
//! > 由 `kb_core` 决定并对外公布的是上边那个名字文件。细节见
//! > [`rendezvous_`](self) 的模块文档。
//!
//! # 契约（`abs_kb_svc` README §5）
//!
//! - **future 里不阻塞**：本 crate 的 await 只等 ipc-channel 的 router 线程
//!   （`to_stream()`）与 `futures_channel::oneshot`；阻塞的 `accept()` /
//!   `recv()` 全部收敛在**调用方提供的阻塞线程**或本 crate 自己的路由线程上。
//! - **取消可达**：客户端代理在等待应答时同时轮询取消令牌，令牌一触发就立刻
//!   收手，不会把调用方永远挂住（已经发出的请求不会撤回，服务端的应答会被丢弃）。
//! - **错误分层**：传输层失败是本 crate 的 [`ServoIpcError`]；业务失败是
//!   [`abs_kb_svc::v1::desktop::RpcError::Business`]，两者在类型上分开。
//!
//! # 示例
//!
//! 完整的两端往返见 `tests/round_trip.rs`；最小形状是：
//!
//! ```no_run
//! use kb_svc_servo_ipc::{Client, Listener};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // 服务端（通常在工作线程上）
//! let listener = Listener::bind("/tmp/kb-demo")?;
//! let connection = listener.accept()?; // 阻塞
//! // connection.serve(&service).await?;  // service: impl TrKbService
//!
//! // 客户端
//! let client = Client::connect("/tmp/kb-demo")?;
//! # let _ = (connection, client);
//! # Ok(())
//! # }
//! ```

#![feature(impl_trait_in_assoc_type)]
// `gen_mcf2` 会把 `async fn` 上**显式声明**的生命周期做成生成类型的泛型参数，
// 所以客户端代理里那些 `'c` 不能省略——省略之后 clippy 的 `needless_lifetimes`
// 反而是错的建议。
#![allow(clippy::needless_lifetimes)]

mod client_;
mod connection_;
mod error_;
mod listener_;
mod rendezvous_;

pub use client_::Client;
pub use connection_::Connection;
pub use error_::ServoIpcError;
pub use listener_::Listener;
pub use rendezvous_::{
    DEFAULT_CONNECT_TIMEOUT, NAME_FILE_EXTENSION, NAME_FILE_PREFIX, find_name_file_in,
    new_name_file_in,
};
