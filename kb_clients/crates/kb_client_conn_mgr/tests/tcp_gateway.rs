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
    AddWorkspaceRequest, AskRequest, CreateSessionRequest, ErrorCode, ErrorReply, LocalId, Reply,
    ReplyEnvelope, Request, RequestEnvelope, ServerInfo, ServiceList, SessionDetail, SessionId,
    SessionList, SessionSummary, Turn, TurnId, TurnState, Workspace, WorkspaceId, WorkspaceList,
};
use abs_llm::v1::cont::Role;
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

/// 造一条消息。
fn turn_(id: &str, role: Role, text: &str) -> Turn {
    Turn {
        turn_id: TurnId::new(id),
        role,
        text: text.to_string(),
        reasoning: String::new(),
        state: TurnState::Done,
        tool_calls: Vec::new(),
        usage: None,
        notice: None,
    }
}

/// 给一个请求配一个应答。
///
/// 假网关只实现"列表 + 工作区 / 会话的增删"这几条，用来覆盖 `KbClient` 的窄接口；
/// 其余请求一律回 `BadRequest`（见 `business_error_is_passed_through_`）。
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
        // 增：把服务端分配的标识编出来，名字 / 路径原样回显，便于断言"真的带过去了"。
        Request::AddWorkspace(request) => Reply::WorkspaceAdded {
            local_id: request.local_id.clone(),
            workspace: Workspace {
                workspace_id: WorkspaceId::new("w-new"),
                name: request.name.clone(),
                path: request.path.clone(),
            },
        },
        Request::RemoveWorkspace { .. } => Reply::Ack,
        Request::RenameWorkspace { workspace_id, name } => {
            Reply::WorkspaceRenamed(Workspace {
                workspace_id: workspace_id.clone(),
                name: name.clone(),
                path: "/tmp/fake".to_string(),
            })
        }
        Request::CreateSession(request) => Reply::SessionCreated {
            local_id: request.local_id.clone(),
            session: SessionSummary {
                session_id: SessionId::new("s-new"),
                workspace_id: request.workspace_id.clone(),
                // 没有显式标题时，假网关也照 `kb_core` 的口径从首条用户消息推导。
                title: request
                    .title
                    .clone()
                    .filter(|title| !title.trim().is_empty())
                    .or_else(|| {
                        request
                            .turns
                            .iter()
                            .find(|turn| turn.role == Role::User)
                            .map(|turn| turn.text.clone())
                    })
                    .unwrap_or_else(|| "新会话".to_string()),
                updated_at_millis: 1_760_000_000_000,
                turn_count: request.turns.len() as u32,
            },
        },
        Request::RemoveSession { .. } => Reply::Ack,
        Request::RenameSession {
            workspace_id,
            session_id,
            title,
        } => Reply::SessionRenamed(SessionSummary {
            session_id: session_id.clone(),
            workspace_id: workspace_id.clone(),
            title: title.clone(),
            updated_at_millis: 1_760_000_000_000,
            turn_count: 2,
        }),
        // 正文与生成：假网关也照 `kb_core` 的口径把问题逆序当回答，好让客户端
        // 侧的映射与断言有真实形状可言。
        Request::GetSession {
            workspace_id,
            session_id,
        } => Reply::SessionDetail(SessionDetail {
            summary: SessionSummary {
                session_id: session_id.clone(),
                workspace_id: workspace_id.clone(),
                title: "假会话".to_string(),
                updated_at_millis: 1_760_000_000_000,
                turn_count: 1,
            },
            turns: vec![turn_("t-1", Role::User, "假问题")],
        }),
        Request::Ask(request) => {
            let answer: String = request.question.chars().rev().collect();
            Reply::SessionDetail(SessionDetail {
                summary: SessionSummary {
                    session_id: request.session_id.clone(),
                    workspace_id: request.workspace_id.clone(),
                    title: "假会话".to_string(),
                    updated_at_millis: 1_760_000_000_000,
                    turn_count: 2,
                },
                turns: vec![
                    turn_(request.turn_id.as_ref(), Role::User, &request.question),
                    turn_("t-answer", Role::Assistant, &answer),
                ],
            })
        }
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
///   （`Request::ListDirectory` 命中它的兜底分支，回 `Reply::Error`）。
/// - 判断：拿到的是 `Reply::Error` 且 code 是 `BadRequest`——协议层把业务失败
///   当成一种**正常应答**，界面据此提示"这条操作不被支持"而不是"掉线了"。
#[test]
fn business_error_is_passed_through_() {
    let address = start_gateway_(Behaviour::Normal);
    let tcp = TcpClient::connect(&address, Duration::from_secs(5)).expect("应当能连上");

    let envelope = block_on(async {
        tcp.send_request(Request::ListDirectory {
            workspace_id: WorkspaceId::new("w-fake"),
            relative_path: String::new(),
        })
        .may_cancel_with(TimeoutToken::after(Duration::from_secs(5)))
        .await
    })
    .expect("传输层应当成功");

    match envelope.reply {
        Reply::Error(reply) => assert_eq!(reply.code, ErrorCode::BadRequest),
        other => panic!("应当是业务错误，实际: {other:?}"),
    }
}

/// 测试远程路径上的工作区 / 会话增删：走的是 `KbClient` 的窄接口。
///
/// - 手段：用 `Connection::tcp` 连上正常行为的假网关；依次调
///   `add_workspace` → `create_session` → `remove_session` → `remove_workspace`。
/// - 判断：新增工作区带回服务端分配的 `w-new`，且名字 / 路径与提交的一致
///   （说明簿记字段 `local_id` 在线上往返了一次但**没有**污染返回值）；
///   新建会话带回 `s-new` 且归属提交的工作区；两次删除都成功。
#[test]
fn tcp_profile_cruds_workspaces_and_sessions_() {
    let address = start_gateway_(Behaviour::Normal);
    let profile = Connection::tcp("假网关", address);

    let client = block_on(async { connect(&profile).await }).expect("应当能连上");
    let timeout = || TimeoutToken::after(Duration::from_secs(5));

    let workspace = block_on(async {
        client
            .add_workspace(AddWorkspaceRequest {
                local_id: LocalId::generate(),
                name: "新建的工作区".to_string(),
                path: "/tmp/created".to_string(),
            })
            .may_cancel_with(timeout())
            .await
    })
    .expect("应当能新增工作区");
    assert_eq!(workspace.workspace_id, WorkspaceId::new("w-new"));
    assert_eq!(workspace.name, "新建的工作区");
    assert_eq!(workspace.path, "/tmp/created");

    let session = block_on(async {
        client
            .create_session(CreateSessionRequest {
                workspace_id: workspace.workspace_id.clone(),
                local_id: LocalId::generate(),
                title: Some("第一问".to_string()),
                turns: Vec::new(),
            })
            .may_cancel_with(timeout())
            .await
    })
    .expect("应当能新建会话");
    assert_eq!(session.session_id, SessionId::new("s-new"));
    assert_eq!(session.workspace_id, workspace.workspace_id);
    assert_eq!(session.title, "第一问");

    block_on(async {
        client
            .remove_session(workspace.workspace_id.clone(), session.session_id.clone())
            .may_cancel_with(timeout())
            .await
    })
    .expect("应当能删除会话");

    block_on(async {
        client
            .remove_workspace(workspace.workspace_id.clone())
            .may_cancel_with(timeout())
            .await
    })
    .expect("应当能删除工作区");
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

/// 测试远程路径上的"读正文 + 提问"。
///
/// - 手段：正常假网关，先 `get_session` 再 `ask`（问题用中文，好验证按字符逆序）。
/// - 判断：`get_session` 拿回一条消息；`ask` 拿回两回合，用户回合保留客户端给的
///   `turn_id`、助手正文是问题的逆序、摘要 `turn_count` 为 2——说明两个请求都被
///   正确编码、按 `SessionDetail` 解回，`Ask` 也确实走了生成域。
#[test]
fn tcp_profile_reads_session_and_asks_() {
    let address = start_gateway_(Behaviour::Normal);
    let profile = Connection::tcp("假网关", address);
    let client = block_on(async { connect(&profile).await }).expect("应当能连上");
    let timeout = || TimeoutToken::after(Duration::from_secs(5));

    let detail = block_on(async {
        client
            .get_session(WorkspaceId::new("w-fake"), SessionId::new("s-fake"))
            .may_cancel_with(timeout())
            .await
    })
    .expect("读会话应当成功");
    assert_eq!(detail.turns.len(), 1);
    assert_eq!(detail.turns[0].text, "假问题");

    let asked = block_on(async {
        client
            .ask(AskRequest {
                workspace_id: WorkspaceId::new("w-fake"),
                session_id: SessionId::new("s-fake"),
                turn_id: TurnId::new("t-9"),
                question: "你好".to_string(),
                service_id: None,
            })
            .may_cancel_with(timeout())
            .await
    })
    .expect("提问应当成功");
    assert_eq!(asked.turns.len(), 2);
    assert_eq!(asked.turns[0].turn_id, TurnId::new("t-9"));
    assert_eq!(asked.turns[0].text, "你好");
    assert_eq!(asked.turns[1].text, "好你");
    assert_eq!(asked.summary.turn_count, 2);
}

/// 测试远程路径上的重命名：工作区与会话各来一次。
///
/// - 手段：正常假网关，`rename_workspace` 与 `rename_session` 各调一次。
/// - 判断：两个调用都拿到改名之后的载荷（新名字 / 新标题），标识保持不变——
///   说明 `RenameWorkspace` / `RenameSession` 被正确编码、按新应答变体解回。
#[test]
fn tcp_profile_renames_workspace_and_session_() {
    let address = start_gateway_(Behaviour::Normal);
    let profile = Connection::tcp("假网关", address);
    let client = block_on(async { connect(&profile).await }).expect("应当能连上");
    let timeout = || TimeoutToken::after(Duration::from_secs(5));

    let workspace = block_on(async {
        client
            .rename_workspace(WorkspaceId::new("w-fake"), "新名字".to_string())
            .may_cancel_with(timeout())
            .await
    })
    .expect("工作区改名应当成功");
    assert_eq!(workspace.workspace_id, WorkspaceId::new("w-fake"));
    assert_eq!(workspace.name, "新名字");

    let session = block_on(async {
        client
            .rename_session(
                WorkspaceId::new("w-fake"),
                SessionId::new("s-fake"),
                "新标题".to_string(),
            )
            .may_cancel_with(timeout())
            .await
    })
    .expect("会话改名应当成功");
    assert_eq!(session.session_id, SessionId::new("s-fake"));
    assert_eq!(session.title, "新标题");
}
