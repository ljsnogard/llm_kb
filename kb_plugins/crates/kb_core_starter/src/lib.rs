//! # kb_core_starter
//!
//! 用命令行启动一个 `kb_core` 子进程，并**异步地**等它公布自己的
//! **IPC 端点文件名**。
//!
//! ```text
//! 调用方 ──Command::new("kb-core") --handshake-prompt=stdio──► kb_core（子进程）
//!        ◄──── stdout 上的一行 IpcReadyNotice（JSON）────────┘
//! ```
//!
//! # 它实现协议里的哪一块
//!
//! 只实现**系统层握手**的"接收方"这一半：起进程 + 收 [`IpcReadyNotice`]。
//! 它**不实现**应用层握手（`TrHandshake` / `Request::Hello`）——那要等真正
//! 连上之后由 IPC 或 TCP 客户端去做，而且不是每个父子关系都需要
//! （`kb_core_rproxy` 就等到真有远程客户端时才做）。
//!
//! 两个层面的消息都是公开协议，只是定义在不同的 crate 里：
//!
//! | 层面 | 消息 | 定义在 | 谁实现 |
//! | :--- | :--- | :--- | :--- |
//! | **系统层**（找得到、连得上） | [`IpcReadyNotice`]（`event: "ipc_ready"`） | **本 crate 依赖的 `abs_kb_core_handshake`** | 本 crate（收）+ `kb_core`（发） |
//! | **应用层**（谈得成） | `Request::Hello` → `Reply::Hello` → `Event::Ready` | `abs_kb_svc_v1_desktop`（经 `abs_kb_svc` 聚合） | `kb_svc_servo_ipc::Client` / 将来的 TCP 客户端 |
//!
//! 系统层的消息单独一个 crate，正是为了让本 crate **不必依赖整套业务协议**
//! （`abs_kb_svc_v1_desktop` 及其背后的 `abs_llm` 等）。本 crate 的依赖里只有
//! 一个协议 crate：[`abs_kb_core_handshake`]。完整理由见 crate 根的 `README.md`。
//!
//! [`IpcReadyNotice`]: abs_kb_core_handshake::IpcReadyNotice
//! [`abs_kb_core_handshake`]: abs_kb_core_handshake
//!
//! # 异步、可取消、与运行时无关
//!
//! [`start`] 返回的是一个**与异步运行时无关**的 future：
//!
//! - **future 里没有阻塞等待**：读那一行通知的 `read_line` 发生在一条专职线程上
//!   （进程级 `std::thread`），future 只轮询一个完成量；因此 tokio / compio /
//!   `futures_lite::block_on` 都能驱动它；
//! - **可以取消**：`start(&spec).may_cancel_with(token).await` 用
//!   [`abs_cancel::TrCancellationToken`] 表达"不再等了"。**等多久由调用方决定**——
//!   本 crate 不自带定时器，也不假定调用方用的是哪个运行时；
//! - **取消/出错/丢弃都会收掉子进程**：只要 `kb_core` 是本 crate 起来的，
//!   它就不会在"等待失败"之后变成孤儿。成功时进程的所有权交给 [`Launched`]，
//!   它在 `Drop` 时结束子进程。
//!
//! # 示例
//!
//! ```no_run
//! use kb_core_starter::{LaunchSpec, start};
//!
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! let spec = LaunchSpec {
//!     kb_core: "/usr/local/bin/kb-core".into(),
//!     runtime_dir: "/run/user/1000/llm_kb".into(),
//!     storage_dir: "/run/user/1000/llm_kb/storage".into(),
//! };
//!
//! // 不可取消：一直等到 kb_core 把端点文件名打出来。
//! let launched = start(&spec).await?;
//! println!("端点名字文件: {}", launched.name_file().display());
//! # Ok(())
//! # }
//! ```
//!
//! 需要"等太久就别等了"时，由调用方造一个取消令牌（带自己的定时器），走可取消路径：
//!
//! ```no_run
//! # use abs_cancel::{TrCancellationToken, TrMayCancel};
//! # use kb_core_starter::{LaunchSpec, start};
//! # async fn demo<C>(spec: &LaunchSpec, timeout_token: C)
//! #     -> Result<(), Box<dyn std::error::Error>>
//! # where
//! #     C: TrCancellationToken,
//! # {
//! let launched = start(spec).may_cancel_with(timeout_token).await?;
//! # let _ = launched;
//! # Ok(())
//! # }
//! ```
//!
//! # 现状与边界
//!
//! - `kb_core` 只有一个 `[[bin]]`，没有 lib target，所以"启动"只能是起进程；
//! - `kb-core` 可执行文件的位置**必须由调用方给出**（[`LaunchSpec::kb_core`]）。
//!   [`default_kb_core_path`] / [`kb_core_beside`] 只是"猜一个"的兜底，对
//!   Flutter 打包出来的 App 并不成立（`current_exe()` 是 Flutter runner）。

// `gen_mcf2` 为「可取消 future」生成的 `TrMayCancel` 关联类型需要它；
// `kb_svc_servo_ipc` 与 `kb_core` 出于同样的理由开着这个特性。
#![feature(impl_trait_in_assoc_type)]
// 宏会把 `async fn` 上**显式声明**的生命周期做成生成类型的泛型参数，
// 因此那些生命周期不能省——省了之后 clippy 的 `needless_lifetimes` 反而是错的建议。
#![allow(clippy::needless_lifetimes)]

mod error_;
mod launch_;
mod notice_;

pub use error_::LaunchError;
pub use launch_::{LaunchSpec, Launched, default_kb_core_path, kb_core_beside, start};
