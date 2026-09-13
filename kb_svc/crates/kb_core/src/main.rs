//! # kb_core
//!
//! 知识库的主进程入口。
//!
//! 依据 `dev-notes.md` 的决策（§13.1）：`kb_svc_salvo` 保持为纯库，负责 HTTP /
//! WebSocket 路由与会话逻辑；**进程的启动与编排由本 crate 负责**。现阶段两者被
//! 视为一体，将来再拆分。
//!
//! # 本文件当前状态
//!
//! PoC 阶段：`kb_core` 只负责把 `kb_svc_salvo` 的 PoC 服务端拉起来，用于验证
//! 「Unix domain socket 上能否跑 WebSocket」。正式实现会替换为完整的启动编排
//! （配置文件、插件子进程监管、更多路由）。
//!
//! # 用法
//!
//! ```text
//! kb_core [--runtime-dir <dir>] [tcp_addr]
//! ```
//!
//! - `tcp_addr`：用户侧 HTTP 监听地址，缺省 `127.0.0.1:8788`；端口写 `0` 表示由系统分配。
//! - `--runtime-dir`：存放插件通道 socket 的目录，缺省 `$XDG_RUNTIME_DIR/llm_kb`。
//!
//! 注意：socket **文件名不接受指定**，由 `kb_svc_salvo` 在启动时按「日期 + UUID」
//! 生成；`kb_core` 拿到该路径后负责把它交给插件子进程。
//!
//! 收到 `Ctrl-C`（`SIGINT`）或 `SIGTERM` 时，本进程会走优雅退出路径，
//! 删除本次生成的 socket 文件后再退出（见 [`shutdown_signal`]）。

use std::path::PathBuf;

use kb_svc_salvo::poc::{PocConfig, bind};
use log::info;

/// PoC 默认使用的 HTTP 监听地址。
const DEFAULT_TCP_ADDR: &str = "127.0.0.1:8788";

/// `kb_core` 的命令行参数。
#[derive(Debug, Default, PartialEq, Eq)]
struct Args {
    /// 用户侧 HTTP 监听地址。
    tcp_addr: Option<String>,

    /// 运行时目录；`None` 表示使用 `kb_svc_salvo` 的默认值。
    runtime_dir: Option<PathBuf>,
}

/// 解析命令行参数。
///
/// 参数顺序不敏感，未知参数会被忽略并打印警告。
fn parse_args<I>(argv: I) -> Args
where
    I: IntoIterator<Item = String>,
{
    let mut args = Args::default();
    let mut iter = argv.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--runtime-dir" => match iter.next() {
                Some(dir) => args.runtime_dir = Some(PathBuf::from(dir)),
                None => log::warn!("--runtime-dir 缺少取值，已忽略"),
            },
            other if other.starts_with('-') => {
                log::warn!("忽略未知参数: {other}");
            }
            addr => {
                if args.tcp_addr.is_some() {
                    log::warn!("重复的监听地址参数，已忽略: {addr}");
                } else {
                    args.tcp_addr = Some(addr.to_string());
                }
            }
        }
    }

    args
}

/// 等待进程收到 `SIGINT` / `SIGTERM`。
///
/// 这一步是 socket 文件能被清理的前提：进程若被信号直接杀死，`Drop` 不会执行，
/// 生成的 socket 文件就会残留在运行时目录里。
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut interrupt = match signal(SignalKind::interrupt()) {
            Ok(stream) => stream,
            Err(err) => {
                log::warn!("无法监听 SIGINT，仅等待退出信号: {err}");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };

        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(stream) => stream,
            Err(err) => {
                log::warn!("无法监听 SIGTERM，仅等待 SIGINT: {err}");
                interrupt.recv().await;
                return;
            }
        };

        tokio::select! {
            _ = interrupt.recv() => info!("收到 SIGINT，开始优雅退出"),
            _ = terminate.recv() => info!("收到 SIGTERM，开始优雅退出"),
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        info!("收到中断信号，开始优雅退出");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = parse_args(std::env::args().skip(1));

    let mut config = PocConfig::new(
        args.tcp_addr
            .clone()
            .unwrap_or_else(|| DEFAULT_TCP_ADDR.to_string()),
    );

    if let Some(runtime_dir) = args.runtime_dir.clone() {
        config = config.with_runtime_dir(runtime_dir);
    }

    let mut bound = bind(&config).await?;

    info!("kb_core listening: tcp={}", bound.tcp_addr);
    info!("kb_core plugin socket: {}", bound.socket_path().display());

    // TODO(阶段 3)：在此处启动插件子进程，并把 `bound.socket_path()` 通过
    // `--socket <path>` 与 `LLM_KB_PLUGIN_SOCKET` 两种方式传给子进程。

    // 先把 socket 文件的清理守卫拿到 `main` 的作用域里：服务端任务可能在关停时被
    // `abort`，若守卫留在任务内部，清理会随任务一起被取消，socket 文件就会残留。
    let socket_guard = bound
        .take_socket_guard()
        .ok_or("socket 清理守卫缺失，拒绝继续启动")?;

    let serve_task = tokio::spawn(async move {
        bound.serve().await;
    });

    shutdown_signal().await;

    // 给服务端一点时间自然退出；超时后直接中止任务。
    match tokio::time::timeout(std::time::Duration::from_secs(3), serve_task).await {
        Ok(Ok(())) => info!("服务端已正常退出"),
        Ok(Err(err)) => log::warn!("服务端任务异常结束: {err}"),
        Err(_) => log::warn!("服务端未在 3 秒内退出，已放弃等待"),
    }

    // 显式删除 socket 文件，并给出可观测的结果。
    match socket_guard.remove() {
        Ok(()) => info!("已清理 socket 文件: {}", socket_guard.path().display()),
        Err(err) => log::warn!(
            "清理 socket 文件失败 {}: {err}",
            socket_guard.path().display()
        ),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试参数解析正确识别监听地址与运行时目录。
    ///
    /// - 手段：把一组参数以 `String` 迭代器的形式传给 `parse_args`，顺序打乱以验证
    ///   解析与顺序无关。
    /// - 判断：返回的 `Args` 中 `tcp_addr` 与 `runtime_dir` 与预期完全相等。
    #[test]
    fn parse_args_reads_addr_and_runtime_dir() {
        let args = parse_args(
            ["--runtime-dir", "/run/kb", "127.0.0.1:9999"]
                .into_iter()
                .map(String::from),
        );

        assert_eq!(
            args,
            Args {
                tcp_addr: Some("127.0.0.1:9999".to_string()),
                runtime_dir: Some(PathBuf::from("/run/kb")),
            }
        );
    }

    /// 测试缺省参数时两个字段都为空，由调用方补默认值。
    ///
    /// - 手段：传入空参数列表。
    /// - 判断：`Args` 等于 `Default`，即仍使用代码内定义的默认监听地址与默认目录。
    #[test]
    fn parse_args_defaults_to_empty() {
        assert_eq!(parse_args(std::iter::empty()), Args::default());
    }

    /// 测试未知参数与重复地址不会破坏解析结果。
    ///
    /// - 手段：传入一个未知开关、一次重复的监听地址，以及 `--runtime-dir` 缺少取值。
    /// - 判断：第一个监听地址被保留，运行时目录保持为 `None`，解析过程不 panic。
    #[test]
    fn parse_args_tolerates_unknown_and_duplicate() {
        let args = parse_args(
            ["--nope", "127.0.0.1:1", "127.0.0.1:2", "--runtime-dir"]
                .into_iter()
                .map(String::from),
        );

        assert_eq!(
            args,
            Args {
                tcp_addr: Some("127.0.0.1:1".to_string()),
                runtime_dir: None,
            }
        );
    }
}
