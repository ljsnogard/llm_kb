//! 端到端测试：浏览器侧与插件侧两段 WebSocket 经由服务端打通。
//!
//! 这些测试使用真实的 TCP 连接与服务端，覆盖 `dev-notes.md` §14 的验收路径：
//!
//! 1. 打开页面就能收到 `ready`；
//! 2. 没有插件时提问会被明确拒绝；
//! 3. 插件上线后，提问会被转发过去，插件的 `delta` / `finished` 会回到浏览器；
//! 4. 设置接口可以写入服务与 API key，并在读取时遮蔽 key。

use std::{path::PathBuf, sync::Arc, time::Duration};

use futures_util::{SinkExt, StreamExt};
use kb_svc_salvo::{
    hub::AppState,
    server::{ServerConfig, bind},
    settings::{LlmServiceConfig, SettingsStore},
};
use tokio::time::timeout;
use tokio_tungstenite::{
    WebSocketStream, client_async,
    tungstenite::{Message, client::IntoClientRequest},
};

/// 单个操作的超时时间。
const STEP_TIMEOUT: Duration = Duration::from_secs(10);

/// 一个已经启动的测试服务端。
struct TestServer {
    /// 用户侧 HTTP 地址。
    tcp_addr: std::net::SocketAddr,

    /// 插件通道 socket 路径。
    socket_path: PathBuf,

    /// 独立运行时目录。
    runtime_dir: PathBuf,

    /// 后台服务端任务。
    task: tokio::task::JoinHandle<()>,
}

impl TestServer {
    /// 启动一个隔离的服务端（内存设置 + 独立 socket 目录）。
    async fn start(name: &str, store: SettingsStore) -> Self {
        let base =
            std::env::temp_dir().join(format!("kb-svc-salvo-e2e-{}-{name}", std::process::id()));
        let runtime_dir = base.join("run");

        let _ = std::fs::remove_dir_all(&base);

        let state = Arc::new(AppState::new(store).await);
        let config = ServerConfig::new("127.0.0.1:0").with_runtime_dir(runtime_dir.clone());

        let bound = bind(&config).await.expect("应当成功绑定监听器");
        let tcp_addr = bound.tcp_addr;
        let socket_path = bound.socket_path().to_path_buf();

        let task = tokio::spawn(async move {
            bound.serve(state).await;
        });

        tokio::time::sleep(Duration::from_millis(80)).await;

        Self {
            tcp_addr,
            socket_path,
            runtime_dir,
            task,
        }
    }

    /// 连接浏览器侧通道。
    async fn connect_chat(&self) -> WebSocketStream<tokio::net::TcpStream> {
        let stream = tokio::net::TcpStream::connect(self.tcp_addr)
            .await
            .expect("应当能连上 TCP 监听器");
        connect(stream, &format!("ws://{}/ws/chat", self.tcp_addr)).await
    }

    /// 连接插件侧通道（经由 Unix domain socket）。
    async fn connect_plugin(&self) -> WebSocketStream<tokio::net::UnixStream> {
        let stream = tokio::net::UnixStream::connect(&self.socket_path)
            .await
            .expect("应当能连上 UDS 监听器");
        connect(stream, "ws://localhost/ws/plugin").await
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(self.runtime_dir.parent().unwrap_or(&self.runtime_dir));
    }
}

/// 在给定流上完成 WebSocket 握手。
async fn connect<S>(stream: S, uri: &str) -> WebSocketStream<S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let request = uri
        .into_client_request()
        .expect("URI 应当可以构造为握手请求");
    let (socket, response) = client_async(request, stream)
        .await
        .expect("WebSocket 握手应当成功");
    assert_eq!(response.status().as_u16(), 101, "应当返回 101");
    socket
}

/// 读取下一条文本帧并解析成 JSON。
async fn recv_json<S>(socket: &mut WebSocketStream<S>) -> serde_json::Value
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        let message = timeout(STEP_TIMEOUT, socket.next())
            .await
            .expect("读取不应超时")
            .expect("连接不应断开")
            .expect("协议层不应报错");

        match message {
            Message::Text(text) => {
                return serde_json::from_str(&text).expect("服务端应当发出合法 JSON");
            }
            _ => continue,
        }
    }
}

/// 读取下一条「非 ready」文本帧。
///
/// 插件上下线时服务端会向所有页面广播 `ready`，因此断言业务事件时需要跳过它。
async fn recv_event<S>(socket: &mut WebSocketStream<S>) -> serde_json::Value
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        let value = recv_json(socket).await;
        if value["type"] != "ready" {
            return value;
        }
    }
}

/// 发送一条 JSON 文本帧。
async fn send_json<S>(socket: &mut WebSocketStream<S>, value: &serde_json::Value)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .expect("发送应当成功");
}

/// 测试浏览器通道建立后立即收到 `ready`。
///
/// - 手段：连接 `/ws/chat` 并读取第一条消息。
/// - 判断：`type` 为 `ready`，且在没有服务时 `services` 为空数组。
#[tokio::test]
async fn chat_channel_greets_with_ready() {
    let server = TestServer::start("ready", SettingsStore::memory()).await;
    let mut chat = server.connect_chat().await;

    let ready = recv_json(&mut chat).await;
    assert_eq!(ready["type"], "ready");
    assert_eq!(ready["plugin_online"], false);
    assert_eq!(ready["services"], serde_json::json!([]));
}

/// 测试没有插件在线时提问会被拒绝，并回一条 `plugin_offline`。
///
/// - 手段：写入一个带 key 的服务（内存设置），连接浏览器通道后发送 `ask`。
/// - 判断：随后收到的 `error` 帧 `code` 为 `plugin_offline`，且 `turn_id` 回显。
#[tokio::test]
async fn ask_without_plugin_returns_error_frame() {
    let store = SettingsStore::memory();
    store
        .upsert_service(
            "deepseek",
            &LlmServiceConfig::new("deepseek", "deepseek-chat", "", "sk-test"),
        )
        .await
        .expect("写入内存设置应当成功");

    let server = TestServer::start("offline", store).await;
    let mut chat = server.connect_chat().await;
    let ready = recv_json(&mut chat).await;
    assert_eq!(ready["active_service"], "deepseek");

    send_json(
        &mut chat,
        &serde_json::json!({ "type": "ask", "turn_id": "t-offline", "question": "在吗" }),
    )
    .await;

    let error = recv_event(&mut chat).await;
    assert_eq!(error["type"], "error");
    assert_eq!(error["code"], "plugin_offline");
    assert_eq!(error["turn_id"], "t-offline");
}

/// 测试完整的转发链路：提问 → 插件 → 增量回到浏览器。
///
/// - 手段：同时连接浏览器通道与插件通道；浏览器提问后，插件应当收到 `ask`
///   指令；插件依次上报 `started`、两条 `delta`（正文与推理）和 `finished`。
/// - 判断：浏览器按序收到 `started`、`delta(answer)`、`delta(reasoning)`、
///   `finished`，且文本与 turn 标识完全一致。
#[tokio::test]
async fn browser_question_reaches_plugin_and_answers_stream_back() {
    let store = SettingsStore::memory();
    store
        .upsert_service(
            "deepseek",
            &LlmServiceConfig::new("deepseek", "deepseek-chat", "", "sk-test"),
        )
        .await
        .expect("写入内存设置应当成功");

    let server = TestServer::start("roundtrip", store).await;

    let mut chat = server.connect_chat().await;
    let _ready = recv_json(&mut chat).await;

    let mut plugin = server.connect_plugin().await;

    // 插件连上后，服务端会先下发一条 hello。
    let hello = recv_json(&mut plugin).await;
    assert_eq!(hello["type"], "hello");

    send_json(
        &mut chat,
        &serde_json::json!({ "type": "ask", "turn_id": "t1", "question": "介绍一下你自己" }),
    )
    .await;

    // 插件侧应当收到转发过来的 ask，且带着服务配置（含 key）。
    let ask = recv_json(&mut plugin).await;
    assert_eq!(ask["type"], "ask");
    assert_eq!(ask["turn_id"], "t1");
    assert_eq!(ask["question"], "介绍一下你自己");
    assert_eq!(ask["service"]["model"], "deepseek-chat");
    assert_eq!(ask["service"]["api_key"], "sk-test");

    // 插件上报事件。内容分片一律是 `raw` + rig 的原始载荷，
    // 由 `kb_rig_llm_v1_adapt` 在服务端翻译（dev-notes §2.2 / §2.3）。
    for event in [
        serde_json::json!({ "type": "started", "turn_id": "t1", "model": "deepseek-chat" }),
        serde_json::json!({
            "type": "raw",
            "turn_id": "t1",
            "payload": { "type": "reasoningDelta", "id": "r1", "reasoning": "先想一下" }
        }),
        serde_json::json!({
            "type": "raw",
            "turn_id": "t1",
            "payload": { "type": "text", "text": "我是" }
        }),
        serde_json::json!({
            "type": "raw",
            "turn_id": "t1",
            "payload": { "type": "text", "text": "一个助手" }
        }),
        serde_json::json!({
            "type": "raw",
            "turn_id": "t1",
            "payload": { "input_tokens": 12, "output_tokens": 5 }
        }),
        serde_json::json!({ "type": "finished", "turn_id": "t1", "reason": "completed" }),
    ] {
        send_json(&mut plugin, &event).await;
    }

    let started = recv_event(&mut chat).await;
    assert_eq!(started["type"], "started");
    assert_eq!(started["service_id"], "deepseek");
    assert_eq!(started["model"], "deepseek-chat");

    // 字段名与 `abs_llm::v1` 对齐：`logic` + `text`。
    let reasoning = recv_event(&mut chat).await;
    assert_eq!(reasoning["type"], "delta");
    assert_eq!(reasoning["logic"], "reasoning");
    assert_eq!(reasoning["text"], "先想一下");

    let answer1 = recv_event(&mut chat).await;
    assert_eq!(answer1["logic"], "answer");
    assert_eq!(answer1["text"], "我是");

    let answer2 = recv_event(&mut chat).await;
    assert_eq!(answer2["text"], "一个助手");

    // 用量在嵌套的 `usage` 对象里。
    let usage = recv_event(&mut chat).await;
    assert_eq!(usage["type"], "usage");
    assert_eq!(usage["usage"]["total_tokens"], 17);

    let finished = recv_event(&mut chat).await;
    assert_eq!(finished["type"], "finished");
    assert_eq!(finished["reason"], "completed");
}

/// 测试设置接口写入服务后可读回，且 API key 被遮蔽。
///
/// - 手段：用 `fetch` 等价的最小 HTTP 请求（这里直接调用 HTTP 接口）POST 一个
///   服务到 `/api/settings/services`，再 GET `/api/settings`。
/// - 判断：返回的服务里 `has_api_key` 为真、`api_key` 等于遮蔽串而不是明文。
#[tokio::test]
async fn settings_api_round_trip_masks_key() {
    let server = TestServer::start("settings", SettingsStore::memory()).await;

    let body = serde_json::json!({
        "id": "deepseek",
        "provider": "deepseek",
        "model": "deepseek-chat",
        "base_url": "https://api.deepseek.com",
        "api_key": "sk-secret",
    })
    .to_string();

    let response = http_request(
        server.tcp_addr,
        "POST /api/settings/services HTTP/1.1",
        Some(("application/json", &body)),
    )
    .await;
    assert!(response.contains("200 OK"), "写入应当成功: {response}");

    let response = http_request(server.tcp_addr, "GET /api/settings HTTP/1.1", None).await;
    assert!(response.contains("200 OK"), "读取应当成功: {response}");
    assert!(
        response.contains("\"has_api_key\":true"),
        "应当报告已配置 key: {response}"
    );
    assert!(
        !response.contains("sk-secret"),
        "响应不应当含明文 key: {response}"
    );
}

/// 发送一次最小 HTTP/1.1 请求并返回完整响应（含状态行与消息体）。
async fn http_request(
    addr: std::net::SocketAddr,
    request_line: &str,
    body: Option<(&str, &str)>,
) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("应当能连上 TCP 监听器");

    let mut request = format!("{request_line}\r\nHost: {addr}\r\nConnection: close\r\n");

    match body {
        Some((content_type, payload)) => {
            request.push_str(&format!(
                "content-type: {content_type}\r\ncontent-length: {}\r\n\r\n{payload}",
                payload.len()
            ));
        }
        None => request.push_str("\r\n"),
    }

    stream
        .write_all(request.as_bytes())
        .await
        .expect("应当能写出请求");

    let mut response = Vec::new();
    timeout(STEP_TIMEOUT, stream.read_to_end(&mut response))
        .await
        .expect("读取响应不应超时")
        .expect("应当能读取响应");

    String::from_utf8_lossy(&response).into_owned()
}
