//! PoC：验证「Unix domain socket 上能否跑 WebSocket」这一风险点。
//!
//! ## 验证目标
//!
//! `dev-notes.md` §10 风险点 21 指出：Salvo 的 `WebSocketUpgrade` 理论上只依赖
//! HTTP/1.1 的 `Upgrade` 机制、与底层传输无关，但本仓库尚未验证过。本模块把该
//! 疑问压缩成最小的可执行代码：
//!
//! 1. 用 `salvo::conn::unix::UnixListener` 与 `salvo::conn::tcp::TcpListener` 各起一个监听器，
//!    并用 `salvo::conn::JoinedListener` 合并交给同一个 `Server`；
//! 2. 路由表对两个监听器**完全共享**，`GET /` 返回一行文本，`GET /ws` 升级为 WebSocket；
//! 3. WebSocket 处理器把收到的文本消息原样回显（`echo`），以便验证双向收发。
//!
//! socket 路径不由调用方指定，而是由 [`crate::plugin_socket`] 按「日期 + UUID」生成；
//! 绑定成功后通过 [`BoundPoc::socket_path`] 返回，供启动插件子进程时使用。
//!
//! ## 临时性说明
//!
//! 本模块是**一次性验证代码**。PoC 通过后，其中的监听器组装逻辑会被吸收进正式
//! 的 `server` / `web` / `plugin` 模块，届时本模块应当删除。
//!
//! ## 示例
//!
//! ```no_run
//! use kb_svc_salvo::poc::{PocConfig, run};
//!
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! // 运行时目录缺省为 `$XDG_RUNTIME_DIR/llm_kb`。
//! let config = PocConfig::new("127.0.0.1:0");
//! run(config).await?;
//! # Ok(())
//! # }
//! ```

use std::path::{Path, PathBuf};

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

/// PoC 服务端的配置。
#[derive(Debug, Clone)]
pub struct PocConfig {
    /// 用户侧 HTTP 服务的监听地址，端口为 0 时表示由系统分配临时端口。
    tcp_addr: String,

    /// 运行时目录；`None` 表示使用 [`plugin_socket::default_runtime_dir`]。
    runtime_dir: Option<PathBuf>,
}

impl PocConfig {
    /// 构造一份 PoC 配置。
    ///
    /// - `tcp_addr`：HTTP 监听地址，例如 `127.0.0.1:8788` 或 `127.0.0.1:0`。
    ///
    /// socket 文件名由本模块在绑定时生成，**不接受调用方指定**。
    pub fn new(tcp_addr: impl Into<String>) -> Self {
        Self {
            tcp_addr: tcp_addr.into(),
            runtime_dir: None,
        }
    }

    /// 指定运行时目录（用于测试或部署时把 socket 放到别处）。
    ///
    /// 注意：这里只覆盖**目录**；文件名仍然由「日期 + UUID」生成。
    pub fn with_runtime_dir(mut self, runtime_dir: impl Into<PathBuf>) -> Self {
        self.runtime_dir = Some(runtime_dir.into());
        self
    }

    /// 解析出本次启动实际使用的运行时目录。
    fn resolved_runtime_dir(&self) -> PathBuf {
        self.runtime_dir
            .clone()
            .unwrap_or_else(plugin_socket::default_runtime_dir)
    }
}

/// 已经绑定但尚未开始服务的 PoC 服务端。
///
/// 持有 acceptor、TCP 实际地址、生成的 socket 路径以及 socket 文件的清理守卫。
pub struct BoundPoc {
    /// 合并后的监听器。
    pub acceptor: JoinedAcceptor<UnixAcceptor, TcpAcceptor>,

    /// TCP 侧实际绑定的地址（`127.0.0.1:0` 时由系统分配）。
    pub tcp_addr: std::net::SocketAddr,

    /// 本次启动生成的 Unix domain socket 路径。
    socket_path: PathBuf,

    /// socket 文件的清理守卫。
    ///
    /// 默认随 `BoundPoc` 一同析构；调用方若希望把清理时机绑定到自己的作用域
    /// （例如「服务端任务退出后再删文件」），可以用 [`BoundPoc::take_socket_guard`]
    /// 把它取走。
    guard: Option<SocketFileGuard>,
}

impl BoundPoc {
    /// 返回本次启动生成的 Unix domain socket 路径。
    ///
    /// 启动插件子进程时，应当把该路径通过命令行参数与环境变量传给子进程。
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// 取走 socket 文件的清理守卫。
    ///
    /// 取走之后，socket 文件的删除时机由调用方掌控：守卫被 drop 时删除文件。
    /// 典型用法是在 `main` 里先取走守卫、再把 `BoundPoc` 交给 `serve` 或在任务中运行，
    /// 这样即使服务端任务被 `abort`，文件清理也不会被连带取消。
    pub fn take_socket_guard(&mut self) -> Option<SocketFileGuard> {
        self.guard.take()
    }

    /// 开始提供服务（会一直阻塞）。
    pub async fn serve(self) {
        let acceptor = self.acceptor;
        Server::new(acceptor).serve(router()).await;
    }
}

/// 构造 PoC 的路由表。
///
/// 该路由表不区分「用户侧」与「插件侧」，两个监听器共用同一份，
/// 这样 PoC 可以在 TCP 上做常规验证、在 UDS 上做关键验证。
pub fn router() -> Router {
    Router::new()
        .get(index)
        .push(Router::with_path("ws").goal(ws_echo))
}

/// `GET /`：返回一行说明文本，用于验证普通 HTTP 请求在两个监听器上都可达。
#[handler]
async fn index() -> &'static str {
    "kb_svc_salvo poc: GET /ws to open a websocket\n"
}

/// `GET /ws`：升级为 WebSocket 并回显收到的文本消息。
#[handler]
async fn ws_echo(req: &mut Request, res: &mut Response) {
    let upgrade = WebSocketUpgrade::new();

    let result = upgrade
        .upgrade(req, res, |socket| async move {
            handle_socket(socket).await;
        })
        .await;

    if let Err(err) = result {
        log::warn!("websocket upgrade failed: {err}");
    }
}

/// WebSocket 会话主循环：文本原样回显，二进制回显长度，收到 Close 后退出。
async fn handle_socket(mut socket: WebSocket) {
    while let Some(message) = socket.recv().await {
        let message = match message {
            Ok(message) => message,
            Err(err) => {
                log::warn!("websocket receive failed: {err}");
                break;
            }
        };

        log::debug!("websocket received: {:?}", message);

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

/// 完成「生成 socket 路径 + 绑定监听器」这一步。
///
/// 把绑定与 `serve` 拆开，是为了让调用方（测试、示例、`kb_core`）能在服务端开始
/// 服务之前就拿到 TCP 真实端口与 socket 路径，从而可以：
///
/// 1. 用 `127.0.0.1:0` 取得系统分配的临时端口；
/// 2. 用生成的 socket 路径去启动插件子进程。
pub async fn bind(config: &PocConfig) -> KbSvcResult<BoundPoc> {
    let runtime_dir = config.resolved_runtime_dir();
    plugin_socket::ensure_runtime_dir(&runtime_dir)?;

    let socket_path = plugin_socket::generate_for_today(&runtime_dir);

    let unix = UnixListener::new(socket_path.clone());
    let tcp = TcpListener::new(config.tcp_addr.clone());

    let acceptor = JoinedListener::new(unix, tcp).bind().await;

    plugin_socket::restrict_socket_permissions(&socket_path)?;

    let tcp_addr = tcp_socket_addr(&acceptor)
        .ok_or_else(|| KbSvcError::Server("TCP 监听地址不可用".to_string()))?;

    log::info!(
        "kb_svc_salvo poc bound: tcp={tcp_addr} uds={}",
        socket_path.display()
    );

    Ok(BoundPoc {
        acceptor,
        tcp_addr,
        socket_path: socket_path.clone(),
        guard: Some(SocketFileGuard::new(socket_path)),
    })
}

/// 运行 PoC 服务端（绑定 + 服务，会一直阻塞）。
pub async fn run(config: PocConfig) -> KbSvcResult<()> {
    bind(&config).await?.serve().await;

    Ok(())
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
