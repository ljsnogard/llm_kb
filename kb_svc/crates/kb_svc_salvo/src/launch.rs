//! 服务启动编排：配置加载、监听绑定、优雅退出与资源清理。
//!
//! # 这一层为什么存在
//!
//! `kb_core` 是可执行入口，但它**不应该**知道 socket 清理守卫、信号处理、
//! 配置文件模板这些细节——那些都属于本 crate 的实现。因此这里对外只暴露：
//!
//! - [`LaunchConfig`]：调用方（`kb_core`）描述「用哪个配置、监听哪里、资源在哪」；
//! - [`launch`]：一个异步函数，跑完整个生命周期，正常退出后返回。
//!
//! 调用方拿到的东西只有「成功 / 失败」：
//!
//! ```no_run
//! use kb_svc_salvo::launch::{LaunchConfig, launch};
//!
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! launch(LaunchConfig::new()).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # 退出流程
//!
//! ```text
//! SIGINT / SIGTERM                      服务端任务
//!       │                                   │
//!       ├── 收到信号 ──────────────────────► 等待退出（最多 SHUTDOWN_GRACE）
//!       │                                   │
//!       └── 删除本次启动的 socket 文件 ◄──────┘
//! ```
//!
//! socket 文件的删除由本模块负责，而不是把守卫交给调用方：守卫一旦被移出本 crate，
//! 调用方就有机会把它放进会被 `abort` 的任务里，清理也就随之失效。

use std::{path::PathBuf, sync::Arc, time::Duration};

use log::{info, warn};

use crate::{
    hub::AppState,
    server::{ServerConfig, bind},
    settings::{SettingsStore, default_config_path},
};

/// 关停时等待服务端退出的时间。
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

/// 启动服务所需的全部参数。
///
/// 所有字段都有缺省值（见 [`LaunchConfig::new`]），因此 `kb_core` 只需要覆盖它
/// 从命令行拿到的那几项。
#[derive(Debug, Clone, Default)]
pub struct LaunchConfig {
    /// 用户侧 HTTP 监听地址；空表示用默认值。
    tcp_addr: Option<String>,

    /// 配置文件路径；`None` 表示用 [`default_config_path`]。
    config_path: Option<PathBuf>,

    /// 插件通道 socket 的目录；`None` 表示用 [`crate::plugin_socket::default_runtime_dir`]。
    runtime_dir: Option<PathBuf>,

    /// 前端资源覆盖目录；`None` 表示只用编译期内嵌资源。
    assets_dir: Option<PathBuf>,
}

impl LaunchConfig {
    /// 构造一份全默认的启动配置。
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置用户侧监听地址。
    pub fn with_tcp_addr(mut self, addr: impl Into<String>) -> Self {
        self.tcp_addr = Some(addr.into());
        self
    }

    /// 设置配置文件路径。
    pub fn with_config_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(path.into());
        self
    }

    /// 设置插件通道 socket 的目录。
    pub fn with_runtime_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.runtime_dir = Some(dir.into());
        self
    }

    /// 设置前端资源覆盖目录。
    pub fn with_assets_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.assets_dir = Some(dir.into());
        self
    }

    /// 已设置的监听地址；`None` 表示使用默认值。
    pub fn tcp_addr(&self) -> Option<&str> {
        self.tcp_addr.as_deref()
    }

    /// 已设置的配置文件路径；`None` 表示使用默认路径。
    pub fn config_path(&self) -> Option<&std::path::Path> {
        self.config_path.as_deref()
    }

    /// 已设置的运行时目录；`None` 表示使用默认目录。
    pub fn runtime_dir(&self) -> Option<&std::path::Path> {
        self.runtime_dir.as_deref()
    }

    /// 已设置的资源覆盖目录；`None` 表示只用内嵌资源。
    pub fn assets_dir(&self) -> Option<&std::path::Path> {
        self.assets_dir.as_deref()
    }

    /// 解析出实际使用的监听地址。
    fn resolved_tcp_addr(&self) -> String {
        self.tcp_addr
            .clone()
            .unwrap_or_else(|| crate::server::DEFAULT_TCP_ADDR.to_string())
    }

    /// 解析出实际使用的配置文件路径。
    fn resolved_config_path(&self) -> PathBuf {
        self.config_path.clone().unwrap_or_else(default_config_path)
    }
}

/// 跑完整个服务生命周期：加载配置 → 绑定监听 → 等待退出信号 → 清理资源。
///
/// 该函数会一直阻塞，直到收到 `SIGINT` / `SIGTERM`（或非 Unix 平台的等价中断）。
/// 正常退出时返回 `Ok(())`，此时本次启动生成的 socket 文件已经被删除。
pub async fn launch(config: LaunchConfig) -> crate::error::KbSvcResult<()> {
    // ── 1. 用户配置：不存在就生成模板，然后读出来 ────────────────────
    let config_path = config.resolved_config_path();
    let store = SettingsStore::file(&config_path);
    store.ensure_exists().await?;

    info!("用户配置: {}", config_path.display());
    log_services(&store).await;

    // ── 2. 共享状态与监听器 ─────────────────────────────────────────
    let state = Arc::new(AppState::new(store).await);

    let mut server_config = ServerConfig::new(config.resolved_tcp_addr());
    if let Some(dir) = config.runtime_dir.clone() {
        server_config = server_config.with_runtime_dir(dir);
    }
    if let Some(dir) = config.assets_dir.clone() {
        server_config = server_config.with_assets_dir(dir);
    }

    let mut bound = bind(&server_config).await?;

    info!("用户界面: http://{}", bound.tcp_addr);
    info!("插件通道: {}", bound.socket_path().display());

    // TODO(阶段 4)：在此处启动 kb_rig_llm 子进程，并把 `bound.socket_path()` 通过
    // `--socket <path>` 与 `LLM_KB_PLUGIN_SOCKET` 两种方式传给子进程。

    // ── 3. 取走清理守卫：它必须留在本函数的作用域里 ────────────────────
    //
    // 服务端任务在关停时可能被 `abort`；守卫若留在任务内部，清理会随之取消，
    // socket 文件就会残留。因此这里先把它拿到外层。
    let socket_guard = bound
        .take_socket_guard()
        .ok_or_else(|| crate::error::KbSvcError::Server("socket 清理守卫缺失".to_string()))?;

    let serve_task = tokio::spawn(async move {
        bound.serve(state).await;
    });

    // ── 4. 等待退出信号，然后回收 ───────────────────────────────────
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

/// 把已配置的服务打到日志里，便于确认「key 是否真的读到了」。
async fn log_services(store: &SettingsStore) {
    let settings = match store.load().await {
        Ok(settings) => settings,
        Err(err) => {
            warn!("读取用户配置失败: {err}");
            return;
        }
    };

    if settings.services.is_empty() {
        info!("尚未配置任何 LLM 服务，可在网页右上角「设置」里添加");
        return;
    }

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

/// 等待进程收到 `SIGINT` / `SIGTERM`。
///
/// 这是 socket 文件能被清理的前提：进程若被信号直接杀死（`SIGKILL`），
/// `Drop` 不会执行，生成的 socket 文件就会残留在运行时目录里。
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试缺省配置能解析出约定的默认值。
    ///
    /// - 手段：用 `LaunchConfig::new()` 构造，读取两个解析后的字段。
    /// - 判断：监听地址等于 [`crate::server::DEFAULT_TCP_ADDR`]；
    ///   配置路径等于 [`default_config_path`]。
    #[test]
    fn defaults_resolve_to_documented_values() {
        let config = LaunchConfig::new();

        assert_eq!(config.resolved_tcp_addr(), crate::server::DEFAULT_TCP_ADDR);
        assert_eq!(config.resolved_config_path(), default_config_path());
    }

    /// 测试每个 setter 都会覆盖对应的默认值。
    ///
    /// - 手段：四个字段都显式设置一遍，再读取解析结果。
    /// - 判断：`tcp_addr` 与 `config_path` 返回设置值；`runtime_dir` / `assets_dir`
    ///   作为原始字段被保留（它们只在 `launch` 内部消费）。
    #[test]
    fn setters_override_defaults() {
        let config = LaunchConfig::new()
            .with_tcp_addr("127.0.0.1:0")
            .with_config_path("/tmp/x.toml")
            .with_runtime_dir("/tmp/run")
            .with_assets_dir("/tmp/assets");

        assert_eq!(config.resolved_tcp_addr(), "127.0.0.1:0");
        assert_eq!(config.resolved_config_path(), PathBuf::from("/tmp/x.toml"));
        assert_eq!(config.runtime_dir, Some(PathBuf::from("/tmp/run")));
        assert_eq!(config.assets_dir, Some(PathBuf::from("/tmp/assets")));
    }
}
