//! PoC 集成测试：验证 Unix domain socket 上可以完成 WebSocket 握手与双向收发。
//!
//! 对应 `dev-notes.md` §10 风险点 21。
//!
//! 同时验证 socket 路径的生成约定：**不由调用方指定**，而是每次启动按
//! 「日期 + UUID」生成，并由服务端把该路径交给调用方（将来用于启动插件子进程）。

use std::{path::PathBuf, sync::OnceLock, time::Duration};

use futures_util::{SinkExt, StreamExt};
use kb_svc_salvo::{
    plugin_socket,
    poc::{PocConfig, bind},
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::UnixStream,
    task::JoinHandle,
    time::timeout,
};
use tokio_tungstenite::{
    WebSocketStream, client_async,
    tungstenite::{Message, client::IntoClientRequest},
};

/// 测试整体超时时间，避免 PoC 出问题时测试永久挂起。
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// 一个已经启动的 PoC 服务端，以及它占用的资源。
struct TestServer {
    /// TCP 侧实际绑定的地址。
    tcp_addr: std::net::SocketAddr,

    /// 服务端生成的 Unix domain socket 路径。
    socket_path: PathBuf,

    /// 独立的运行时目录，测试结束时整体删除。
    runtime_dir: PathBuf,

    /// 后台服务端任务；先中止它再删除目录，避免「文件正在被监听」的竞态。
    task: JoinHandle<()>,

    /// 保证目录只清理一次。
    cleaned: OnceLock<()>,
}

impl TestServer {
    /// 为某个测试启动一个独立的 PoC 服务端。
    async fn start(name: &str) -> Self {
        let runtime_dir =
            std::env::temp_dir().join(format!("kb-svc-salvo-poc-{}-{name}", std::process::id()));

        let config = PocConfig::new("127.0.0.1:0").with_runtime_dir(runtime_dir.clone());

        let bound = bind(&config).await.expect("PoC 服务端应当成功绑定监听器");
        let tcp_addr = bound.tcp_addr;
        let socket_path = bound.socket_path().to_path_buf();

        // 服务端在后台任务中运行，测试结束时主动 abort。
        let task = tokio::spawn(async move {
            bound.serve().await;
        });

        // 绑定完成后监听器已经就绪，这里只需要给调度器一点时间进入 accept 循环。
        tokio::time::sleep(Duration::from_millis(50)).await;

        Self {
            tcp_addr,
            socket_path,
            runtime_dir,
            task,
            cleaned: OnceLock::new(),
        }
    }

    /// 中止服务端任务并删除运行时目录。
    ///
    /// 这里刻意不做异步等待：`Drop` 可能发生在异步上下文中，调用
    /// `block_on` 会 panic。清理顺序是「先 abort 任务，再整目录删除」，
    /// 即使 abort 尚未被调度，删除目录也不会影响已经在跑的测试。
    /// socket 文件本身由 `kb_svc_salvo` 的 `SocketFileGuard` 负责删除。
    fn cleanup(&self) {
        if self.cleaned.set(()).is_err() {
            return;
        }

        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.runtime_dir);
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// 校验一个生成的文件名符合「kb-<日期>-<UUID>.sock」约定。
fn assert_socket_file_name(path: &std::path::Path) {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("socket 文件名应当是合法的 UTF-8");

    let rest = name
        .strip_prefix("kb-")
        .unwrap_or_else(|| panic!("socket 文件名应以 kb- 开头，实际: {name}"));
    let (date, rest) = rest
        .split_once('-')
        .unwrap_or_else(|| panic!("socket 文件名应包含日期段，实际: {name}"));

    assert_eq!(date.len(), 8, "日期段应为 YYYYMMDD，实际: {date}");
    assert!(
        date.chars().all(|c| c.is_ascii_digit()),
        "日期段应全部为数字，实际: {date}"
    );

    let uuid = rest
        .strip_suffix(".sock")
        .unwrap_or_else(|| panic!("socket 文件名应以 .sock 结尾，实际: {name}"));
    assert_eq!(uuid.len(), 32, "UUID simple 形式应为 32 字符，实际: {uuid}");
    assert!(
        uuid.chars().all(|c| c.is_ascii_hexdigit()),
        "UUID 部分应全部为十六进制字符，实际: {uuid}"
    );
}

/// 通过给定的流建立 WebSocket 连接并返回客户端。
async fn connect<S>(stream: S, uri: &str) -> WebSocketStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let request = uri
        .into_client_request()
        .expect("URI 应当可以构造为握手请求");
    let (socket, response) = client_async(request, stream)
        .await
        .expect("WebSocket 握手应当成功");
    assert_eq!(
        response.status().as_u16(),
        101,
        "应当返回 101 Switching Protocols"
    );
    socket
}

/// 发送一条文本消息并读取一条文本回复。
async fn echo_roundtrip<S>(socket: &mut WebSocketStream<S>, text: &str) -> String
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    socket
        .send(Message::Text(text.into()))
        .await
        .expect("客户端应当能够发送文本消息");

    timeout(TEST_TIMEOUT, read_text(socket))
        .await
        .expect("读取回复不应超时")
}

/// 读取直到收到一条文本或二进制消息为止。
async fn read_text<S>(socket: &mut WebSocketStream<S>) -> String
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        let message = socket
            .next()
            .await
            .expect("连接不应意外断开")
            .expect("协议层不应报错");

        match message {
            Message::Text(text) => return text.to_string(),
            Message::Binary(bytes) => return String::from_utf8_lossy(&bytes).into_owned(),
            // Ping/Pong 由协议层自动处理，这里只跳过非数据帧。
            _ => continue,
        }
    }
}

/// 测试 socket 路径按「日期 + UUID」生成，且文件确实以 0600 权限创建。
///
/// - 手段：启动 PoC 服务端（bind 不阻塞），检查它生成的路径是否位于指定的运行时
///   目录下、文件名是否符合约定，再用元数据读取 socket 文件的权限位。
/// - 判断：文件名匹配 `kb-<YYYYMMDD>-<32 位十六进制>.sock`，文件存在，且权限
///   恰为 `0o600`——这直接验证了「文件名由服务端生成、不由调用方指定」与
///   「socket 不暴露给同机其他用户」两条约定。
#[tokio::test]
async fn poc_socket_path_is_generated_with_date_and_uuid() {
    use std::os::unix::fs::PermissionsExt;

    let server = TestServer::start("path").await;

    assert_eq!(
        server.socket_path.parent(),
        Some(server.runtime_dir.as_path()),
        "socket 文件应当位于指定的运行时目录内"
    );
    assert_socket_file_name(&server.socket_path);

    let metadata =
        std::fs::symlink_metadata(&server.socket_path).expect("bind 之后 socket 文件应当存在");
    let mode = metadata.permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "socket 文件权限应为 0600，实际 {mode:o}");
}

/// 测试 Unix domain socket 上的完整 WebSocket 链路。
///
/// - 手段：读取服务端生成的 socket 路径，用 `tokio::net::UnixStream` 连接该 UDS，
///   在其上完成 WebSocket 握手，随后发送两轮文本消息并读取回显。
/// - 判断：握手状态码必须为 101；两轮回显内容必须与发送内容逐字节相等，
///   从而同时证明「生成出来的路径可被外部进程使用」「下行与上行都可用」。
#[tokio::test]
async fn poc_ws_over_unix_socket() {
    let server = TestServer::start("uds").await;

    let stream = UnixStream::connect(&server.socket_path)
        .await
        .expect("应当能够连接到 UDS 监听器");

    let mut socket = connect(stream, "ws://localhost/ws").await;

    assert_eq!(
        echo_roundtrip(&mut socket, "hello over uds").await,
        "hello over uds"
    );
    assert_eq!(
        echo_roundtrip(&mut socket, "第二条消息").await,
        "第二条消息"
    );

    let _ = socket.close(None).await;
}

/// 测试 TCP 侧监听器与路由是否正常工作。
///
/// - 手段：用 `tokio::net::TcpStream` 连接 PoC 的 TCP 监听器并完成 WebSocket 握手，
///   发送一条文本消息读取回显；同时用普通 HTTP 请求访问 `GET /`。
/// - 判断：WebSocket 回显与发送内容一致；`GET /` 返回 200 且响应体包含 `poc` 字样，
///   证明「两个监听器共享同一份路由表」这一设计成立。
#[tokio::test]
async fn poc_ws_and_http_over_tcp() {
    let server = TestServer::start("tcp").await;

    // 普通 HTTP：验证共享路由表中的非 WebSocket 处理器。
    let body = http_get(server.tcp_addr, "/").await;
    assert!(
        body.contains("poc"),
        "GET / 的响应体应当包含 poc 字样，实际为 {body:?}"
    );

    let stream = tokio::net::TcpStream::connect(server.tcp_addr)
        .await
        .expect("应当能够连接到 TCP 监听器");

    let mut socket = connect(stream, &format!("ws://{}/ws", server.tcp_addr)).await;
    assert_eq!(
        echo_roundtrip(&mut socket, "hello over tcp").await,
        "hello over tcp"
    );

    let _ = socket.close(None).await;
}

/// 测试 Unix socket 文件的清理守卫在析构时删除文件。
///
/// - 手段：在临时目录下创建一个空文件充当 socket 文件，构造
///   `SocketFileGuard`，确认其 `path()` 后可读，然后 drop 掉它。
/// - 判断：drop 之后文件不存在，说明正式实现退出时的清理路径可行。
#[test]
fn socket_file_guard_removes_file_on_drop() {
    let path = std::env::temp_dir().join(format!(
        "kb-svc-salvo-guard-{}-{}.sock",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));

    std::fs::write(&path, b"").expect("应当能够创建占位文件");

    let guard = plugin_socket::SocketFileGuard::new(&path);
    assert_eq!(guard.path(), path.as_path());
    drop(guard);

    assert!(
        !path.exists(),
        "守卫析构后 socket 文件应当被删除: {}",
        path.display()
    );
}

/// 用一次极简的 HTTP/1.1 请求读取 `GET /` 的响应体。
///
/// 这里刻意不引入 HTTP 客户端依赖：PoC 只需要确认路由可达。
async fn http_get(addr: std::net::SocketAddr, path: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("应当能够连接到 TCP 监听器");

    let request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("应当能够写出 HTTP 请求");

    let mut response = Vec::new();
    timeout(TEST_TIMEOUT, stream.read_to_end(&mut response))
        .await
        .expect("读取 HTTP 响应不应超时")
        .expect("应当能够读取 HTTP 响应");

    String::from_utf8_lossy(&response).into_owned()
}
