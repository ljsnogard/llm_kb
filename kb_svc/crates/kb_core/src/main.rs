//! # kb_core
//!
//! 知识库的主进程入口。
//!
//! 依据 `dev-notes.md` 的决策（§13.1）：`kb_svc_salvo` 保持为纯库，负责 HTTP /
//! WebSocket 路由与会话逻辑；**进程的启动与编排由本 crate 负责**。现阶段两者被
//! 视为一体，将来再拆分。
//!
//! # 职责
//!
//! 1. 解析命令行参数；
//! 2. 读取（必要时创建）用户配置文件，其中包含 LLM 服务选项与 API key；
//! 3. 组装 `kb_svc_salvo::server`，绑定 TCP + Unix socket 双监听器；
//! 4. 处理好关停：收到 `SIGINT` / `SIGTERM` 时清理 socket 文件。
//!
//! 插件子进程的拉起在下一个阶段实现（见 `dev-notes.md` §11）。
//!
//! # 用法
//!
//! ```text
//! kb_core [--config <file>] [--runtime-dir <dir>] [--assets-dir <dir>] [tcp_addr]
//! ```
//!
//! - `tcp_addr`：用户侧 HTTP 监听地址，缺省 `127.0.0.1:8788`；端口写 `0` 表示由系统分配。
//! - `--config`：配置文件路径，缺省 `$XDG_CONFIG_HOME/llm_kb/config.toml`。
//! - `--runtime-dir`：插件通道 socket 的存放目录，缺省 `$XDG_RUNTIME_DIR/llm_kb`。
//! - `--assets-dir`：前端资源覆盖目录（开发期用），缺省使用编译期内嵌资源。
//!
//! 注意：socket **文件名不接受指定**，由 `kb_svc_salvo` 在启动时按「日期 + UUID」
//! 生成；`kb_core` 拿到该路径后负责把它交给插件子进程。

use std::{path::PathBuf, sync::Arc, time::Duration};

use kb_svc_salvo::{
    hub::AppState,
    server::{ServerConfig, bind},
    settings::SettingsStore,
};
use log::{info, warn};

/// 默认使用的 HTTP 监听地址。
const DEFAULT_TCP_ADDR: &str = "127.0.0.1:8788";

/// 关停时等待服务端退出的时间。
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

/// `kb_core` 的命令行参数。
#[derive(Debug, Default, PartialEq, Eq)]
struct Args {
    /// 用户侧 HTTP 监听地址。
    tcp_addr: Option<String>,

    /// 配置文件路径。
    config_path: Option<PathBuf>,

    /// 运行时目录。
    runtime_dir: Option<PathBuf>,

    /// 前端资源覆盖目录。
    assets_dir: Option<PathBuf>,
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
            "--config" => match iter.next() {
                Some(value) => args.config_path = Some(PathBuf::from(value)),
                None => warn!("--config 缺少取值，已忽略"),
            },
            "--runtime-dir" => match iter.next() {
                Some(value) => args.runtime_dir = Some(PathBuf::from(value)),
                None => warn!("--runtime-dir 缺少取值，已忽略"),
            },
            "--assets-dir" => match iter.next() {
                Some(value) => args.assets_dir = Some(PathBuf::from(value)),
                None => warn!("--assets-dir 缺少取值，已忽略"),
            },
            other if other.starts_with('-') => warn!("忽略未知参数: {other}"),
            addr => {
                if args.tcp_addr.is_some() {
                    warn!("重复的监听地址参数，已忽略: {addr}");
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
                warn!("无法监听 SIGINT，退回仅等待 Ctrl-C: {err}");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };

        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(stream) => stream,
            Err(err) => {
                warn!("无法监听 SIGTERM，仅等待 SIGINT: {err}");
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

    // ── 用户配置 ───────────────────────────────────────────────────
    let config_path = args
        .config_path
        .clone()
        .unwrap_or_else(SettingsStore::default_config_path);

    let store = SettingsStore::file(&config_path);
    store.ensure_exists().await?;
    info!("用户配置: {}", config_path.display());

    let settings = store.load().await?;
    if settings.services.is_empty() {
        info!("尚未配置任何 LLM 服务，可在网页右上角「设置」里添加");
    } else {
        for (id, service) in &settings.services {
            info!(
                "服务 {id}: provider={} model={} api_key={}",
                service.provider,
                service.model,
                if service.has_api_key() {
                    "已配置"
                } else {
                    "缺失"
                }
            );
        }
    }

    // ── 服务端 ─────────────────────────────────────────────────────
    let state = Arc::new(AppState::new(store).await);

    let mut server_config = ServerConfig::new(
        args.tcp_addr
            .clone()
            .unwrap_or_else(|| DEFAULT_TCP_ADDR.to_string()),
    );

    if let Some(dir) = args.runtime_dir.clone() {
        server_config = server_config.with_runtime_dir(dir);
    }
    if let Some(dir) = args.assets_dir.clone() {
        server_config = server_config.with_assets_dir(dir);
    }

    let mut bound = bind(&server_config).await?;

    info!("用户界面: http://{}", bound.tcp_addr);
    info!("插件通道: {}", bound.socket_path().display());

    // TODO(阶段 3)：在此处启动 kb_rig_llm 子进程，并把 `bound.socket_path()` 通过
    // `--socket <path>` 与 `LLM_KB_PLUGIN_SOCKET` 两种方式传给子进程。

    // 先把 socket 文件的清理守卫拿到 `main` 的作用域里：服务端任务可能在关停时被
    // `abort`，若守卫留在任务内部，清理会随任务一起被取消，socket 文件就会残留。
    let socket_guard = bound
        .take_socket_guard()
        .ok_or("socket 清理守卫缺失，拒绝继续启动")?;

    let serve_task = tokio::spawn(async move {
        bound.serve(state).await;
    });

    shutdown_signal().await;

    match tokio::time::timeout(SHUTDOWN_GRACE, serve_task).await {
        Ok(Ok(())) => info!("服务端已正常退出"),
        Ok(Err(err)) => warn!("服务端任务异常结束: {err}"),
        Err(_) => warn!("服务端未在 {SHUTDOWN_GRACE:?} 内退出，已放弃等待"),
    }

    match socket_guard.remove() {
        Ok(()) => info!("已清理 socket 文件: {}", socket_guard.path().display()),
        Err(err) => warn!(
            "清理 socket 文件失败 {}: {err}",
            socket_guard.path().display()
        ),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试参数解析正确识别全部四个选项。
    ///
    /// - 手段：把一组参数以 `String` 迭代器的形式传给 `parse_args`，顺序打乱以验证
    ///   解析与顺序无关。
    /// - 判断：返回的 `Args` 与预期完全相等。
    #[test]
    fn parse_args_reads_all_options() {
        let args = parse_args(
            [
                "--assets-dir",
                "/tmp/assets",
                "127.0.0.1:9999",
                "--config",
                "/tmp/kb.toml",
                "--runtime-dir",
                "/run/kb",
            ]
            .into_iter()
            .map(String::from),
        );

        assert_eq!(
            args,
            Args {
                tcp_addr: Some("127.0.0.1:9999".to_string()),
                config_path: Some(PathBuf::from("/tmp/kb.toml")),
                runtime_dir: Some(PathBuf::from("/run/kb")),
                assets_dir: Some(PathBuf::from("/tmp/assets")),
            }
        );
    }

    /// 测试缺省参数时四个字段都为空，由调用方补默认值。
    ///
    /// - 手段：传入空参数列表。
    /// - 判断：`Args` 等于 `Default`。
    #[test]
    fn parse_args_defaults_to_empty() {
        assert_eq!(parse_args(std::iter::empty()), Args::default());
    }

    /// 测试未知参数与重复地址不会破坏解析结果。
    ///
    /// - 手段：传入一个未知开关、两次重复的监听地址，以及缺少取值的 `--config`。
    /// - 判断：第一个监听地址被保留，`config_path` 保持为 `None`，解析过程不 panic。
    #[test]
    fn parse_args_tolerates_unknown_and_duplicate() {
        let args = parse_args(
            ["--nope", "127.0.0.1:1", "127.0.0.1:2", "--config"]
                .into_iter()
                .map(String::from),
        );

        assert_eq!(
            args,
            Args {
                tcp_addr: Some("127.0.0.1:1".to_string()),
                config_path: None,
                runtime_dir: None,
                assets_dir: None,
            }
        );
    }
}
