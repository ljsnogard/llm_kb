//! PoC 验证代码（**临时**）：`dev-notes.md` §10 风险点 26 的证据留存。
//!
//! # 它验证过什么
//!
//! 「Salvo 的 WebSocket 升级能否在 Unix domain socket 上工作」。结论是**可以**，
//! 详见 `dev-notes.md` §13.5 与集成测试 `tests/poc_uds_websocket.rs`。
//!
//! # 为什么还留着
//!
//! 正式实现已经迁到 [`crate::server`] / [`crate::web`] / [`crate::plugin`]，
//! 本模块只剩两个用途：
//!
//! 1. [`bind_echo_fixture`]：给 `tests/poc_uds_websocket.rs` 提供「不依赖 `kb_core`、
//!    纯回显」的最小服务端，让那条历史证据仍然可重复执行；
//! 2. 保留「双监听器合并」这段最小代码作为后来者的参考。
//!
//! 当测试不再需要它时，本模块可以直接删除。

use salvo::{
    conn::{
        Acceptor, JoinedAcceptor, JoinedListener,
        tcp::TcpAcceptor,
        unix::{UnixAcceptor, UnixListener},
    },
    prelude::*,
    websocket::{Message, WebSocket, WebSocketUpgrade},
};

use crate::{
    error::{KbSvcError, KbSvcResult},
    plugin_socket::{self, SocketFileGuard},
};

/// 测试夹具的配置。
#[derive(Debug, Clone)]
pub struct FixtureConfig {
    /// 用户侧 TCP 监听地址。
    tcp_addr: String,

    /// 运行时目录；`None` 表示使用默认值。
    runtime_dir: Option<std::path::PathBuf>,
}

impl FixtureConfig {
    /// 构造一份夹具配置。
    pub fn new(tcp_addr: impl Into<String>) -> Self {
        Self {
            tcp_addr: tcp_addr.into(),
            runtime_dir: None,
        }
    }

    /// 指定运行时目录。
    pub fn with_runtime_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.runtime_dir = Some(dir.into());
        self
    }
}

/// 已经绑定但尚未服务的夹具。
pub struct BoundFixture {
    /// 合并后的监听器。
    pub acceptor: JoinedAcceptor<UnixAcceptor, TcpAcceptor>,

    /// TCP 侧实际绑定的地址。
    pub tcp_addr: std::net::SocketAddr,

    /// 生成的 socket 路径。
    socket_path: std::path::PathBuf,

    /// socket 文件清理守卫。
    _guard: SocketFileGuard,
}

impl BoundFixture {
    /// 生成的 Unix domain socket 路径。
    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    /// 开始服务（回显路由），会一直阻塞。
    pub async fn serve(self) {
        Server::new(self.acceptor).serve(echo_router()).await;
    }
}

/// 夹具的路由：`GET /` 返回一行文本，`GET /ws` 回显。
pub fn echo_router() -> Router {
    Router::new()
        .get(index)
        .push(Router::with_path("ws").goal(ws_echo))
}

/// `GET /`：一行说明文本。
#[handler]
async fn index() -> &'static str {
    "kb_svc_salvo poc: GET /ws to open a websocket\n"
}

/// `GET /ws`：升级为 WebSocket 并回显文本。
#[handler]
async fn ws_echo(req: &mut Request, res: &mut Response) {
    let result = WebSocketUpgrade::new()
        .upgrade(req, res, |socket| async move {
            echo_loop(socket).await;
        })
        .await;

    if let Err(err) = result {
        log::warn!("websocket upgrade failed: {err}");
    }
}

/// 回显循环。
async fn echo_loop(mut socket: WebSocket) {
    while let Some(message) = socket.recv().await {
        let message = match message {
            Ok(message) => message,
            Err(err) => {
                log::warn!("websocket receive failed: {err}");
                break;
            }
        };

        if message.is_close() {
            break;
        }

        let reply = if let Ok(text) = message.as_str() {
            Message::text(text)
        } else {
            Message::text(format!("binary:{}", message.as_bytes().len()))
        };

        if let Err(err) = socket.send(reply).await {
            log::warn!("websocket send failed: {err}");
            break;
        }
    }
}

/// 绑定夹具的双监听器。
pub async fn bind_echo_fixture(config: &FixtureConfig) -> KbSvcResult<BoundFixture> {
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

    let tcp_addr = acceptor
        .holdings()
        .iter()
        .find_map(|holding| match &holding.local_addr {
            salvo::conn::SocketAddr::IPv4(addr) => Some(std::net::SocketAddr::V4(*addr)),
            salvo::conn::SocketAddr::IPv6(addr) => Some(std::net::SocketAddr::V6(*addr)),
            _ => None,
        })
        .ok_or_else(|| KbSvcError::Server("TCP 监听地址不可用".to_string()))?;

    Ok(BoundFixture {
        acceptor,
        tcp_addr,
        socket_path: socket_path.clone(),
        _guard: SocketFileGuard::new(socket_path),
    })
}
