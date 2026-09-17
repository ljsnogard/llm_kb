//! # kb_core
//!
//! 知识库**主进程**。
//!
//! 本 crate 负责：
//!
//! 1. 解析命令行参数（[`args_`]）；
//! 2. 准备运行时目录，并打开工作区 / 会话的本地文件存储（[`store_`]）；
//! 3. 把存储接上 IPC（[`ipc_`]），常驻服务客户端（[`serve_`]）；
//! 4. 另外提供一组**临时**的 CRUD 子命令（[`cli_`]），便于不开客户端时手工检验。
//!
//! # 异步运行时是 compio
//!
//! 本进程曾由 `kb_svc_salvo` 启动、跑在 tokio 上。现在两者都已移除：
//! HTTP / WebSocket 那套通道整体废弃，进程间通信改由 `kb_svc_servo_ipc`
//! 承担；运行时换成 [compio](https://crates.io/crates/compio)，
//! 本地文件读写走 `compio::fs`，阻塞的 `accept()` 走 `spawn_blocking`。
//!
//! # 工作区与会话存在哪里
//!
//! 暂时用本地文件代替 Turso：数据放在 `--storage-dir`（默认 `<运行时目录>/storage`），
//! 布局与约定见 [`store_`] 的模块文档。文件内容就是
//! `abs_kb_svc::v1::desktop` 里协议类型的 JSON——存储格式与线上格式同源。
//!
//! # 客户端怎么找到它
//!
//! 服务端启动时在 `<运行时目录>` 下生成一个「日期 + UUID」命名的端点名字文件
//! （`kb-<YYYYMMDD>-<uuid>.ipc`，由 `kb_core` 决定）；客户端
//! （`kb_svc_servo_ipc::Client`）在运行时目录里找到它并连上来，见
//! [`kb_svc_servo_ipc::new_name_file_in`] 与 [`serve_`] 的模块文档。
//!
//! # 用法
//!
//! ```text
//! kb-core [--runtime-dir <目录>] [--storage-dir <目录>] [<子命令>]
//! ```
//!
//! 完整说明见 [`args_::USAGE`]，操作手册见本 crate 的 `README.md`。

// `kb_svc_servo_ipc` 与 `ipc_` 里的 `gen_mcf2` 展开产物需要它。
#![feature(impl_trait_in_assoc_type)]
// `gen_mcf2` 会把 `async fn` 上**显式声明**的生命周期做成生成类型的泛型参数，
// 所以 `ipc_` 里那些 `'s` 不能省略——省略之后 clippy 的 `needless_lifetimes`
// 反而是错的建议。
#![allow(clippy::needless_lifetimes)]

mod args_;
mod cli_;
mod error_;
mod ipc_;
mod serve_;
mod store_;

use std::process::ExitCode;

use log::error;

use args_::{Command, Parsed};
use error_::CoreError;

/// 进程入口。
///
/// 无子命令时进入常驻服务（[`serve_::run`]）；带子命令时执行一次增删查改后退出
/// （[`cli_::run_workspace`] / [`cli_::run_session`]）。
///
/// 退出码：`0` 成功，`1` 运行期失败，`2` 参数错误（常驻服务被信号终止时由信号决定）。
#[compio::main]
async fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let parsed = match args_::parse(std::env::args().skip(1)) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("参数错误: {error}");
            eprintln!("{}", args_::USAGE);
            return ExitCode::from(2);
        }
    };

    match dispatch_(parsed).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// 按解析结果分发。
///
/// 单独拆出来是为了让 `main` 只保留"日志 + 退出码"这件事；同时它也是唯一
/// 同时用到 [`serve_`] 与 [`cli_`] 的地方。
async fn dispatch_(parsed: Parsed) -> Result<(), CoreError> {
    match parsed.command {
        Command::Help => {
            print!("{}", args_::USAGE);
            Ok(())
        }
        Command::Serve => serve_::run(&parsed.paths, parsed.handshake_prompt).await,
        Command::Workspace(command) => Ok(cli_::run_workspace(&parsed.paths, command).await?),
        Command::Session(command) => Ok(cli_::run_session(&parsed.paths, command).await?),
    }
}
