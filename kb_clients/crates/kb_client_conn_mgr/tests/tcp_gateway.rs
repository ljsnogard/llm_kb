//! 用**假网关**验证连接管理器的远程路径。
//!
//! 假网关是一个进程内的 `TcpListener`，说的是 `kb_core_rproxy` 的帧协议
//! （编解码直接用 `kb_core_rproxy_wire`，与真网关共用同一份定义）。
//! 这样测试不依赖任何子进程，也能精确构造"坏帧""慢应答"这些不好复现的情况。
//!
//! 真网关 + 真 `kb_core` 的端到端验证不在这里：见 `examples/connect.rs`
//! （手工跑，或者在有 `kb-core` 二进制的环境里跑）。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use abs_cancel::{NonCancellableToken, TrMayCancel};
use abs_kb_svc_v1_desktop::{
    AddWorkspaceRequest, ErrorCode, ErrorReply, LocalId, Reply, ReplyEnvelope, Request,
    RequestEnvelope, ServerInfo, ServiceList, SessionId, SessionList, SessionSummary, Workspace,
    WorkspaceId, WorkspaceList,
};
use futures_lite::future::block_on;
use kb_client_config::Connection;
use kb_client_conn_mgr::{TcpClient, TimeoutToken, connect};
use kb_core_rproxy_wire::{Frame, decode_frame, encode_reply};

/// 假网关的行为。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    /// 正常应答每一个请求。
    Normal,

    /// 第一个请求先睡一会儿再答（用来测取消）。
    Slow,

    /// 对第一个请求回一帧长度对不上的垃圾（用来测坏帧）。
    Garbage,
}

/// 起一个假网关，返回它的地址（连接线程随测试结束而结束）。
fn start_gateway_(behaviour: Behaviour) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("应当能绑定端口");
    let address = listener.local_addr().expect("应当有地址").to_string();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { break };
            std::thread::spawn(move || serve_(stream, behaviour));
        }
    });

    address
}

/// 伺候一条连接：按请求回对应的应答。
fn serve_(mut stream: TcpStream, behaviour: Behaviour) {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut first = true;

    loop {
        let read = match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        buffer.extend_from_slice(&chunk[..read]);

        loop {
            let frame = match decode_frame(&mut buffer) {
                Ok(Some(frame)) => frame,
                Ok(None) => break,
                Err(_) => return,
            };
            let Frame::Request(envelope) = frame else {
                return;
            };

            if first && behaviour == Behaviour::Garbage {
                // 长度头说 4 字节，实际只给 1 字节 —— 客户端应当把连接判死。
                let _ = stream.write_all(&[0, 0, 0, 4, 1, 0, 0, 0]);
                let _ = stream.flush();
                return;
            }
            if first && behaviour == Behaviour::Slow {
                std::thread::sleep(Duration::from_millis(800));
            }
            first = false;

            let reply = reply_for_(&envelope);
            let bytes = encode_reply(&reply).expect("应当能编码");
            if stream.write_all(&bytes).is_err() {
                return;
            }
            let _ = stream.flush();
        }
    }
}

/// 给一个请求配一个应答。
fn reply_for_(envelope: &RequestEnvelope) -> ReplyEnvelope {
    let reply = match &envelope.request {
        Request::Hello(_) => Reply::Hello(ServerInfo {
            server_version: "9.9.9".to_string(),
            protocol_version: 1,
        }),
        Request::ListWorkspaces => Reply::WorkspaceList(WorkspaceList {
            workspaces: vec![Workspace {
                workspace_id: WorkspaceId::new("w-fake"),
                name: "假网关的工作区".to_string(),
                path: "/tmp/fake".to_string(),
            }],
        }),
        Request::ListSessions { .. } => Reply::SessionList(SessionList {
            sessions: vec![SessionSummary {
                session_id: SessionId::new("s-fake"),
                workspace_id: WorkspaceId::new("w-fake"),
                title: "假会话".to_string(),
                updated_at_millis: 1_760_000_000_000,
                turn_count: 2,
            }],
        }),
        Request::ListServices => Reply::ServiceList(ServiceList::default()),
        _ => Reply::Error(ErrorReply {
            code: ErrorCode::BadRequest,
            message: "假网关不认这条请求".to_string(),
        }),
    };
    ReplyEnvelope::new(envelope.request_id.clone(), reply)
}

/// 测试远程路径：连上 → 应用层握手 → 列工作区 → 列会话。
///
/// - 手段：起一个正常行为的假网关，用 `Connection::tcp` 组装连接方式，
///   `block_on` 驱动 `connect(...)`（带 5 秒超时令牌）再依次查询。
/// - 判断：握手拿到的服务端版本是假网关写的 `9.9.9`（说明握手真的走了协议）；
///   工作区与会话各拿到 1 条且字段正确；`is_local()` 是 `false`。
#[test]
fn tcp_profile_connects_and_lists_() {
    let address = start_gateway_(Behaviour::Normal);
    let profile = Connection::tcp("假网关", address);

    let client = block_on(async {
        connect(&profile)
            .may_cancel_with(TimeoutToken::after(Duration::from_secs(5)))
            .await
    })
    .expect("应当能连上");

    assert_eq!(client.server_info().server_version, "9.9.9");
    assert!(!client.is_local());
    assert_eq!(client.launched_pid(), None);

    let workspaces = block_on(async {
        client
            .list_workspaces()
            .may_cancel_with(TimeoutToken::after(Duration::from_secs(5)))
            .await
    })
    .expect("应当能列工作区");
    assert_eq!(workspaces.workspaces.len(), 1);
    assert_eq!(workspaces.workspaces[0].name, "假网关的工作区");

    let sessions = block_on(async {
        client
            .list_sessions(workspaces.workspaces[0].workspace_id.clone())
            .may_cancel_with(TimeoutToken::after(Duration::from_secs(5)))
            .await
    })
    .expect("应当能列会话");
    assert_eq!(sessions.sessions.len(), 1);
    assert_eq!(sessions.sessions[0].title, "假会话");
    assert_eq!(sessions.sessions[0].turn_count, 2);
}

/// 测试业务错误原样透传：服务端说"不行"不等于连接坏了。
///
/// - 手段：直接借道底层 `TcpClient`，发一条假网关会拒绝的请求
///   （`Request::AddWorkspace` 命中它的兜底分支，回 `Reply::Error`）。
/// - 判断：拿到的是 `Reply::Error` 且 code 是 `BadRequest`——协议层把业务失败
///   当成一种**正常应答**，界面据此提示"这条操作不被支持"而不是"掉线了"。
#[test]
fn business_error_is_passed_through_() {
    let address = start_gateway_(Behaviour::Normal);
    let tcp = TcpClient::connect(&address, Duration::from_secs(5)).expect("应当能连上");

    let envelope = block_on(async {
        tcp.send_request(Request::AddWorkspace(AddWorkspaceRequest {
            local_id: LocalId::new("l-1"),
            name: "不该成功".to_string(),
            path: "/tmp/x".to_string(),
        }))
        .may_cancel_with(TimeoutToken::after(Duration::from_secs(5)))
        .await
    })
    .expect("传输层应当成功");

    match envelope.reply {
        Reply::Error(reply) => assert_eq!(reply.code, ErrorCode::BadRequest),
        other => panic!("应当是业务错误，实际: {other:?}"),
    }
}

/// 测试取消：应答还没回来就取消，拿到 `ClientError::Cancelled`。
///
/// - 手段：假网关对第一个请求先睡 800ms 再答；客户端用 50ms 的超时令牌。
/// - 判断：结果是 `Err(Cancelled)`（而不是一直等到 800ms 后拿到正常应答），
///   而且 `is_cancelled()` 为真——界面据此区分"超时"与"服务端拒绝"。
#[test]
fn cancelling_the_wait_returns_cancelled_() {
    let address = start_gateway_(Behaviour::Slow);
    let profile = Connection::tcp("慢网关", address);

    let outcome = block_on(async {
        let client = connect(&profile)
            .may_cancel_with(TimeoutToken::after(Duration::from_millis(50)))
            .await?;
        client
            .list_workspaces()
            .may_cancel_with(TimeoutToken::after(Duration::from_millis(50)))
            .await
    });

    match outcome {
        Err(error) => assert!(error.is_cancelled(), "实际: {error:?}"),
        Ok(_) => panic!("50ms 的取消令牌应当先生效"),
    }
}

/// 测试坏帧会把连接判死，而不是把垃圾当应答解。
///
/// - 手段：假网关对第一个请求回一段长度对不上的字节。
/// - 判断：拿到的是传输层失败（`is_transport()` 为真），不是"解出了一个奇怪的应答"。
#[test]
fn malformed_reply_is_reported_as_transport_failure_() {
    let address = start_gateway_(Behaviour::Garbage);
    let profile = Connection::tcp("坏网关", address);

    let outcome = block_on(async {
        let client = connect(&profile)
            .may_cancel_with(TimeoutToken::after(Duration::from_secs(5)))
            .await?;
        client.list_workspaces().await
    });

    match outcome {
        Err(error) => assert!(error.is_transport(), "应当是传输失败，实际: {error:?}"),
        Ok(_) => panic!("坏帧不该被当成正常应答"),
    }
}

/// 测试没给超时令牌也能连（不可取消路径）。
///
/// - 手段：`connect` 之后直接 `.await`（生成的 `IntoFuture` 用
///   `NonCancellableToken`），再显式用一次"永不取消"的令牌查一次。
/// - 判断：两条路径都成功——宏生成的两个入口都没漏。
#[test]
fn non_cancellable_path_also_works_() {
    let address = start_gateway_(Behaviour::Normal);
    let profile = Connection::tcp("假网关", address);

    let client = block_on(async { connect(&profile).await }).expect("应当能连上");
    assert_eq!(client.server_info().protocol_version, 1);

    let listed = block_on(async {
        client
            .list_workspaces()
            .may_cancel_with(NonCancellableToken::new())
            .await
    })
    .expect("应当能列工作区");
    assert_eq!(listed.workspaces.len(), 1);
}
