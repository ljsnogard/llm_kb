//! # kb_client_conn_mgr
//!
//! `kb_admin_desktop` 的**连接管理器**：按一条连接方式（[`kb_client_config::Connection`]）
//! 把客户端接上 `kb_core`，然后暴露一组窄接口（工作区 / 会话的增删查）。
//!
//! ```text
//! 读配置（kb_client_config）
//!    └─ 首次运行：界面问用户怎么连 → 写配置
//! 连接（本 crate）
//!    ├─ ① 系统层握手：起本机 kb_core / 附着到本机 kb_core / 连远程网关
//!    └─ ② 应用层握手：Request::Hello → Reply::Hello
//! 工作区
//!    ├─ list_workspaces()
//!    ├─ add_workspace(request)      目录是 kb_core 所在主机上的路径
//!    └─ remove_workspace(id)        服务端级联删除它名下的会话
//! 会话
//!    ├─ list_sessions(workspace_id)
//!    ├─ create_session(request)
//!    ├─ remove_session(workspace_id, session_id)
//!    ├─ get_session(workspace_id, session_id)   完整正文
//!    └─ ask(request)                            同步一问一答（见 TrGeneration）
//! ```
//!
//! # 三种连接方式
//!
//! | `kind` | 系统层怎么走 | 说明 |
//! | :--- | :--- | :--- |
//! | `local-launch` | `kb_core_starter` 起子进程 + 读就绪通知，再连 IPC | 客户端自己持有子进程，`KbClient` 被丢弃时它被结束 |
//! | `local-attach` | 直接连已经在跑的本机 IPC | 服务端由别人（手工 / rproxy）拉起时用 |
//! | `tcp` | 连 `kb_core_rproxy` 的 TCP 端口 | 跨机；**无鉴权，仅受信网络** |
//!
//! # 与异步运行时无关，且**超时由调用方决定**
//!
//! 所有异步入口都用 `gen_mcf2::gen_may_cancel_future` 展开成
//! 「不可取消 / 可取消」两条路径，future 里没有任何阻塞调用（阻塞的
//! `Client::connect` / `TcpClient::connect` 都搬到了专职线程上）。因此
//! tokio / compio / `futures_lite::block_on` 都能驱动它。
//!
//! 本 crate **不自带定时器**：等多久算超时是调用方的策略。它提供
//! [`TimeoutToken`] 这个"到点就取消"的令牌，调用方按配置里的时限造一个即可：
//!
//! ```no_run
//! # use abs_cancel::TrMayCancel;
//! # use kb_client_config::Connection;
//! # use kb_client_conn_mgr::{TimeoutToken, connect};
//! # async fn demo(profile: &Connection) -> Result<(), Box<dyn std::error::Error>> {
//! // 起进程 / 连端点的时限来自配置；握手与查询共用它也就够了。
//! let token = TimeoutToken::after(profile.handshake_timeout());
//! let client = connect(profile).may_cancel_with(token).await?;
//!
//! let token = TimeoutToken::after(client.request_timeout());
//! let workspaces = client.list_workspaces().may_cancel_with(token).await?;
//! println!("{} 个工作区", workspaces.workspaces.len());
//! # Ok(())
//! # }
//! ```
//!
//! # 它不做什么
//!
//! - **不读配置文件**：那是 [`kb_client_config`] 的事，本 crate 只接受一条已经
//!   选好的 [`kb_client_config::Connection`]；
//! - **不做界面**：FRB 那一层在这里之上再包一遍扁平的 DTO；
//! - **不实现应用层的其它域**（设置 / 目录 / 生成）：协议那边还没落地。

// `gen_mcf2` 为「可取消 future」生成的 `TrMayCancel` 关联类型需要它。
#![feature(impl_trait_in_assoc_type)]
// 宏会把 `async fn` 上**显式声明**的生命周期做成生成类型的泛型参数，
// 因此那些生命周期不能省——省了之后 clippy 的 `needless_lifetimes` 反而是错的建议。
#![allow(clippy::needless_lifetimes)]

mod client_;
mod error_;
mod tcp_;
mod timeout_;

pub use client_::{CLIENT_NAME, KbClient, connect, suggested_local_launch};

// 为了让调用方少钉一遍 `abs_cancel` 的 git 依赖：这两个 trait 是使用本 crate
// 公开异步接口的必需品（`.may_cancel_with(…)`），从这里转出即可。
pub use abs_cancel::{TrCancellationToken, TrMayCancel};
pub use error_::ClientError;
pub use tcp_::{TcpClient, TcpError};
pub use timeout_::TimeoutToken;
