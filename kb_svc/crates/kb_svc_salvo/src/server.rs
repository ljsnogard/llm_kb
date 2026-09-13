//! 服务端组装：路由表、双监听器与共享状态注入。
//!
//! # 两个监听器，一份路由
//!
//! [`bind`] 同时绑定：
//!
//! - **TCP**：给浏览器用（`GET /`、`/app.js`、`/api/settings`、`/ws/chat`）；
//! - **Unix domain socket**：给 `kb_rig_llm` 插件子进程用（`/ws/plugin`）。
//!
//! 两者共用 [`router`] 返回的同一份路由表，因此插件也能访问浏览器侧的路由
//! （当前无实际用途，但省去了「两套路由容易走偏」的维护成本）；反过来浏览器
//! 若通过 TCP 访问 `/ws/plugin` 也会被接受，这与当前「本机单用户、不设防」的
//! 阶段假设一致，正式版本需要按监听器区分权限。
//!
//! # 为什么可以用 `JoinedListener`
//!
//! 「UDS 上能否跑 WebSocket」曾经是本项目最大的未知数，最小验证见 [`crate::poc`]
//! 与集成测试 `tests/poc_uds_websocket.rs`：结论是**可以**——Salvo 的 WebSocket
//! 升级只依赖 HTTP/1.1 的 `Upgrade` 机制，与底层传输类型无关。因此这里的两个
//! 监听器可以直接合并，共享同一套 handler 与状态，不需要为插件通道单独写一套协议。
//!
//! # 监听地址与权限
//!
//! - TCP 侧默认只绑 `127.0.0.1`（见 [`DEFAULT_TCP_ADDR`]），不对外网暴露；
//! - UDS 侧由 [`crate::plugin_socket`] 生成「日期 + UUID」文件名，并把目录收紧到
//!   `0700`、socket 文件收紧到 `0600`。

use std::sync::Arc;

use salvo::{
    conn::{
        Acceptor, JoinedAcceptor, JoinedListener,
        tcp::TcpAcceptor,
        unix::{UnixAcceptor, UnixListener},
    },
    prelude::*,
};

use crate::{
    assets::AssetSettings,
    error::{KbSvcError, KbSvcResult},
    hub::AppState,
    plugin_socket::{self, SocketFileGuard},
};

/// 用户侧 TCP 监听地址的默认值。
pub const DEFAULT_TCP_ADDR: &str = "127.0.0.1:8788";

/// 服务端启动所需的配置。
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// 用户侧 HTTP 监听地址，端口为 0 时表示由系统分配临时端口。
    tcp_addr: String,

    /// 运行时目录；`None` 表示使用 [`plugin_socket::default_runtime_dir`]。
    runtime_dir: Option<std::path::PathBuf>,

    /// 前端资源覆盖目录；`None` 表示只用内嵌资源。
    assets_dir: Option<std::path::PathBuf>,
}

impl ServerConfig {
    /// 构造一份配置。
    ///
    /// socket 文件名由本模块在绑定时生成，**不接受调用方指定**。
    pub fn new(tcp_addr: impl Into<String>) -> Self {
        Self {
            tcp_addr: tcp_addr.into(),
            runtime_dir: None,
            assets_dir: None,
        }
    }

    /// 指定运行时目录（只覆盖**目录**，文件名规则不变）。
    pub fn with_runtime_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.runtime_dir = Some(dir.into());
        self
    }

    /// 指定前端资源覆盖目录（开发期用，见 [`crate::assets`]）。
    pub fn with_assets_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.assets_dir = Some(dir.into());
        self
    }
}

/// 构造完整的路由表（浏览器侧 + 插件侧）。
pub fn router() -> Router {
    Router::new()
        .push(crate::web::router())
        .push(crate::plugin::router())
}

/// 已经绑定但尚未开始服务的服务端。
pub struct BoundServer {
    /// 合并后的监听器。
    acceptor: JoinedAcceptor<UnixAcceptor, TcpAcceptor>,

    /// TCP 侧实际绑定的地址。
    pub tcp_addr: std::net::SocketAddr,

    /// 本次启动生成的 socket 路径。
    socket_path: std::path::PathBuf,

    /// socket 文件的清理守卫。
    guard: Option<SocketFileGuard>,

    /// 资源设置，随服务端一起注入。
    assets: AssetSettings,
}

impl BoundServer {
    /// 返回本次启动生成的 Unix domain socket 路径。
    ///
    /// 启动插件子进程时应当把该路径传给子进程。
    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    /// 取走 socket 文件的清理守卫。
    ///
    /// 取走后，删除时机由调用方掌控（守卫 drop 时删除）。这样即使服务端任务被
    /// `abort`，文件清理也不会被连带取消。
    pub fn take_socket_guard(&mut self) -> Option<SocketFileGuard> {
        self.guard.take()
    }

    /// 注入共享状态并开始提供服务（会一直阻塞）。
    pub async fn serve(self, state: Arc<AppState>) {
        let Self {
            acceptor, assets, ..
        } = self;

        let service = salvo::Service::new(router())
            .hoop(salvo::affix_state::inject(state))
            .hoop(salvo::affix_state::inject(assets));

        Server::new(acceptor).serve(service).await;
    }
}

/// 生成 socket 路径、绑定两个监听器，并完成资源设置。
///
/// 之所以是 `async`：Salvo 的 `Listener::bind` 本身是异步的（它要把监听器注册进
/// 运行时的 reactor）。
pub async fn bind(config: &ServerConfig) -> KbSvcResult<BoundServer> {
    let runtime_dir = config
        .runtime_dir
        .clone()
        .unwrap_or_else(plugin_socket::default_runtime_dir);
    plugin_socket::ensure_runtime_dir(&runtime_dir)?;

    let socket_path = plugin_socket::generate_for_today(&runtime_dir);

    let unix = UnixListener::new(socket_path.clone());
    let tcp = TcpListener::new(config.tcp_addr.clone());

    let acceptor = JoinedListener::new(unix, tcp).bind().await;

    plugin_socket::restrict_socket_permissions(&socket_path)?;

    let tcp_addr = tcp_socket_addr(&acceptor)
        .ok_or_else(|| KbSvcError::Server("TCP 监听地址不可用".to_string()))?;

    let assets = match &config.assets_dir {
        Some(dir) => AssetSettings::with_override_dir(dir.clone()),
        None => AssetSettings::default(),
    };

    if let Some(dir) = assets.override_dir() {
        log::info!("前端资源覆盖目录: {}", dir.display());
    }

    log::info!(
        "kb_svc_salvo 已绑定: tcp={tcp_addr} uds={}",
        socket_path.display()
    );

    Ok(BoundServer {
        acceptor,
        tcp_addr,
        socket_path: socket_path.clone(),
        guard: Some(SocketFileGuard::new(socket_path)),
        assets,
    })
}

/// 从 acceptor 的 holdings 中取出 TCP 侧的实际监听地址。
fn tcp_socket_addr(
    acceptor: &JoinedAcceptor<UnixAcceptor, TcpAcceptor>,
) -> Option<std::net::SocketAddr> {
    acceptor
        .holdings()
        .iter()
        .find_map(|holding| match &holding.local_addr {
            salvo::conn::SocketAddr::IPv4(addr) => Some(std::net::SocketAddr::V4(*addr)),
            salvo::conn::SocketAddr::IPv6(addr) => Some(std::net::SocketAddr::V6(*addr)),
            _ => None,
        })
}
