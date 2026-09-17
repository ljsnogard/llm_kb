//! 把本地文件存储接上 IPC：`kb_core` 这一侧的按域 RPC trait 实现。
//!
//! [`KbService`] 把 [`TrHandshake`] / [`TrWorkspaceService`] / [`TrSessionService`]
//! 的每个方法原样
//! 转发给 [`Store`]，并把 [`StoreError`] 翻成协议里的业务错误：
//!
//! | `StoreError` | `ErrorCode` |
//! | :--- | :--- |
//! | `NotFound` | `NotFound` |
//! | `InvalidId` | `BadRequest` |
//! | `Cancelled` | `Internal`（服务端侧被取消） |
//! | 其它（I/O、解码、阻塞任务） | `Internal` |
//!
//! 一个刻意的选择：[`TrKbEndpoint::Error`] 用 [`Infallible`]——服务端这一侧
//! 不产生"传输层错误"（那是客户端代理的事），所有失败都是业务错误。
//! 派发层里 `RpcError::Transport` 那条分支对本实现不可达。
//!
//! # 关于 `AGENTS.md` 第 4 条
//!
//! 本 crate 因为要展开 [`gen_mcf2::gen_may_cancel_future`]，依赖链上出现了
//! `gen_mcf2`，于是第 4 条落在这里。处理方式是**一路贯到底**：
//!
//! - 本文件里每个方法都由宏展开，且都检查传进来的取消令牌；
//! - 取消令牌**继续往下传**给 [`crate::store_::Store`]
//!   （`.may_cancel_with(cancel)`），而不是在这里丢掉自己造一个；
//! - `Store` 的每个操作同样是宏展开的可取消 future（见那里模块文档），
//!   所以"客户端撤销了一次调用"能一路传到最后一次文件等待。
//!
//! （早先曾以"`kb_core` 是 bin、没有对外库接口"为由只给本文件加取消，
//! 那个判断是错的：只要存在被取消的可能性，就不该从设计上抹掉它。）

use std::convert::Infallible;

use abs_cancel::{TrCancellationToken, TrMayCancel};
use abs_kb_svc::v1::desktop::{
    AddWorkspaceRequest, ClientInfo, CreateSessionRequest, ErrorCode, ErrorReply, PROTOCOL_VERSION,
    RpcError, ServerInfo, SessionDetail, SessionId, SessionList, SessionSummary, TrHandshake,
    TrKbEndpoint, TrSessionService, TrWorkspaceService, Workspace, WorkspaceId, WorkspaceList,
};
use gen_mcf2::gen_may_cancel_future;

use crate::store_::{Store, StoreError};

/// 服务端业务实现。
#[derive(Debug, Clone)]
pub struct KbService {
    /// 底层存储。
    store_: Store,
}

impl KbService {
    /// 用一份存储实现一个服务。
    pub fn new(store: Store) -> Self {
        Self { store_: store }
    }

    /// 底层存储（供日志与测试查看）。
    pub fn store(&self) -> &Store {
        &self.store_
    }
}

/// 把存储失败翻成业务错误应答。
fn store_error_(error: StoreError) -> RpcError<Infallible> {
    let code = match &error {
        StoreError::NotFound { .. } => ErrorCode::NotFound,
        StoreError::InvalidId { .. } => ErrorCode::BadRequest,
        _ => ErrorCode::Internal,
    };
    RpcError::Business(ErrorReply {
        code,
        message: error.to_string(),
    })
}

/// 服务端侧调用被取消时的应答。
///
/// 正常路径不会走到这里：派发层用不可取消的令牌调用这些方法。
/// 保留它是因为"实现不得假定调用者不会取消"（`AGENTS.md` 第 4 条）。
fn cancelled_() -> RpcError<Infallible> {
    RpcError::Business(ErrorReply {
        code: ErrorCode::Internal,
        message: "服务端侧调用被取消".to_string(),
    })
}

// ── 模块级 async fn：宏只能作用于它们 ──────────────────────────────────

/// [`TrHandshake::hello`] 的服务端实现：**应用层握手**的第一步。
///
/// 服务端在这一步只做两件事：校验协议版本、把自己的身份报回去。
/// 它**不**负责"客户端怎么找到我"——那是系统层握手（传输实现）的事，
/// 两者的分工见 `abs_kb_svc::v1::desktop::handshake_` 的模块文档。
#[gen_may_cancel_future(Hello, pub)]
pub async fn hello_async<'s, C>(
    service: &'s KbService,
    client: ClientInfo,
    cancel: C,
) -> Result<ServerInfo, RpcError<Infallible>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(cancelled_());
    }
    let _ = service;

    if client.protocol_version != PROTOCOL_VERSION {
        return Err(RpcError::Business(ErrorReply {
            code: ErrorCode::BadRequest,
            message: format!(
                "协议版本不匹配：客户端说 v{}，服务端是 v{PROTOCOL_VERSION}",
                client.protocol_version
            ),
        }));
    }

    Ok(ServerInfo {
        server_version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: PROTOCOL_VERSION,
    })
}

/// [`TrWorkspaceService::list_workspaces`] 的服务端实现。
#[gen_may_cancel_future(ListWorkspaces, pub)]
pub async fn list_workspaces_async<'s, C>(
    service: &'s KbService,
    cancel: C,
) -> Result<WorkspaceList, RpcError<Infallible>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(cancelled_());
    }
    service
        .store_
        .list_workspaces()
        .may_cancel_with(cancel)
        .await
        .map_err(store_error_)
}

/// [`TrWorkspaceService::add_workspace`] 的服务端实现。
#[gen_may_cancel_future(AddWorkspace, pub)]
pub async fn add_workspace_async<'s, C>(
    service: &'s KbService,
    request: AddWorkspaceRequest,
    cancel: C,
) -> Result<Workspace, RpcError<Infallible>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(cancelled_());
    }
    service
        .store_
        .add_workspace(&request.name, &request.path)
        .may_cancel_with(cancel)
        .await
        .map_err(store_error_)
}

/// [`TrWorkspaceService::remove_workspace`] 的服务端实现。
#[gen_may_cancel_future(RemoveWorkspace, pub)]
pub async fn remove_workspace_async<'s, C>(
    service: &'s KbService,
    workspace_id: WorkspaceId,
    cancel: C,
) -> Result<(), RpcError<Infallible>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(cancelled_());
    }
    service
        .store_
        .remove_workspace(&workspace_id)
        .may_cancel_with(cancel)
        .await
        .map_err(store_error_)
}

/// [`TrSessionService::list_sessions`] 的服务端实现。
#[gen_may_cancel_future(ListSessions, pub)]
pub async fn list_sessions_async<'s, C>(
    service: &'s KbService,
    workspace_id: WorkspaceId,
    cancel: C,
) -> Result<SessionList, RpcError<Infallible>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(cancelled_());
    }
    service
        .store_
        .list_sessions(&workspace_id)
        .may_cancel_with(cancel)
        .await
        .map_err(store_error_)
}

/// [`TrSessionService::create_session`] 的服务端实现。
#[gen_may_cancel_future(CreateSession, pub)]
pub async fn create_session_async<'s, C>(
    service: &'s KbService,
    request: CreateSessionRequest,
    cancel: C,
) -> Result<SessionSummary, RpcError<Infallible>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(cancelled_());
    }
    service
        .store_
        .create_session(&request.workspace_id, request.title, request.turns)
        .may_cancel_with(cancel)
        .await
        .map_err(store_error_)
}

/// [`TrSessionService::get_session`] 的服务端实现。
#[gen_may_cancel_future(GetSession, pub)]
pub async fn get_session_async<'s, C>(
    service: &'s KbService,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    cancel: C,
) -> Result<SessionDetail, RpcError<Infallible>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(cancelled_());
    }
    service
        .store_
        .get_session(&workspace_id, &session_id)
        .may_cancel_with(cancel)
        .await
        .map_err(store_error_)
}

/// [`TrSessionService::remove_session`] 的服务端实现。
#[gen_may_cancel_future(RemoveSession, pub)]
pub async fn remove_session_async<'s, C>(
    service: &'s KbService,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    cancel: C,
) -> Result<(), RpcError<Infallible>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(cancelled_());
    }
    service
        .store_
        .remove_session(&workspace_id, &session_id)
        .may_cancel_with(cancel)
        .await
        .map_err(store_error_)
}

// ── 把生成的 future 填进 trait 的关联类型 ─────────────────────────────

impl TrHandshake for KbService {
    type Hello<'f>
        = HelloAsync<'f, 'f>
    where
        Self: 'f;

    fn hello<'f>(&'f self, client: ClientInfo) -> Self::Hello<'f> {
        HelloAsync::new(self, client)
    }
}

impl TrKbEndpoint for KbService {
    type Error = Infallible;
}

impl TrWorkspaceService for KbService {
    type ListWorkspaces<'f>
        = ListWorkspacesAsync<'f, 'f>
    where
        Self: 'f;
    type AddWorkspace<'f>
        = AddWorkspaceAsync<'f, 'f>
    where
        Self: 'f;
    type RemoveWorkspace<'f>
        = RemoveWorkspaceAsync<'f, 'f>
    where
        Self: 'f;

    fn list_workspaces<'f>(&'f self) -> Self::ListWorkspaces<'f> {
        ListWorkspacesAsync::new(self)
    }

    fn add_workspace<'f>(&'f self, request: AddWorkspaceRequest) -> Self::AddWorkspace<'f> {
        AddWorkspaceAsync::new(self, request)
    }

    fn remove_workspace<'f>(&'f self, workspace_id: WorkspaceId) -> Self::RemoveWorkspace<'f> {
        RemoveWorkspaceAsync::new(self, workspace_id)
    }
}

impl TrSessionService for KbService {
    type ListSessions<'f>
        = ListSessionsAsync<'f, 'f>
    where
        Self: 'f;
    type CreateSession<'f>
        = CreateSessionAsync<'f, 'f>
    where
        Self: 'f;
    type GetSession<'f>
        = GetSessionAsync<'f, 'f>
    where
        Self: 'f;
    type RemoveSession<'f>
        = RemoveSessionAsync<'f, 'f>
    where
        Self: 'f;

    fn list_sessions<'f>(&'f self, workspace_id: WorkspaceId) -> Self::ListSessions<'f> {
        ListSessionsAsync::new(self, workspace_id)
    }

    fn create_session<'f>(&'f self, request: CreateSessionRequest) -> Self::CreateSession<'f> {
        CreateSessionAsync::new(self, request)
    }

    fn get_session<'f>(
        &'f self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> Self::GetSession<'f> {
        GetSessionAsync::new(self, workspace_id, session_id)
    }

    fn remove_session<'f>(
        &'f self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> Self::RemoveSession<'f> {
        RemoveSessionAsync::new(self, workspace_id, session_id)
    }
}

/// 存储错误到业务错误的映射表（`KbService` 模块文档里那张表）。
#[cfg(test)]
mod tests_ {
    use super::*;
    use abs_kb_svc::v1::desktop::{Turn, TurnId, TurnState};
    use kb_svc_servo_ipc::{Client, Listener};

    /// 造一个临时目录对：`(存储目录, 运行时目录)`。
    fn temp_dirs_() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let guard = tempfile::tempdir().expect("应当能创建临时目录");
        let storage = guard.path().join("data");
        let runtime = guard.path().join("run");
        (guard, storage, runtime)
    }

    /// 测试"真存储 + 真 IPC"的整条链路：客户端 → 通道 → 派发 → `Store` → 磁盘。
    ///
    /// - 手段：在独立线程上用 `Listener` 起服务端（业务实现是 [`KbService`]，
    ///   底层是真实目录上的 `Store`）；测试这边用 `Client` 连上去，依次做
    ///   「建工作区 → 列工作区 → 建会话（带一条离线消息）→ 读会话 → 删会话 →
    ///   删工作区」。
    /// - 判断：每一步的业务载荷正确；**并且在服务端进程的存储目录里能直接看到
    ///   对应的 JSON 文件**（说明请求真的落到了磁盘上，而不是在通道里自娱自乐）；
    ///   删除工作区之后它的会话目录整个消失。
    #[compio::test]
    async fn ipc_round_trip_reaches_the_local_store_() {
        let (_guard, storage, runtime) = temp_dirs_();

        // ── 服务端：一条独立线程，独立打开同一份存储 ──────────────────
        let server_storage = storage.clone();
        let server_runtime = runtime.clone();
        let server = std::thread::spawn(move || {
            let store_runtime = compio::runtime::Runtime::new().expect("应当能建运行时");
            // `Store::open` 返回的是 `IntoFuture`（可取消 future）而不是 `Future`，
            // 所以用 async 块包一层再交给 `block_on`。
            let store = store_runtime
                .block_on(async { Store::open(&server_storage).await })
                .expect("服务端应当能打开存储");
            let service = KbService::new(store);

            let listener = Listener::bind(&server_runtime).expect("应当能绑定端点");
            let connection = listener.accept().expect("应当能接受连接");
            store_runtime
                .block_on(connection.serve(&service))
                .expect("serve 不应当失败");
        });

        // ── 客户端 ────────────────────────────────────────────────────
        let client = Client::connect(&runtime).expect("应当能连上服务端");

        let workspace = client
            .add_workspace(AddWorkspaceRequest {
                local_id: "l-1".into(),
                name: "笔记".to_string(),
                path: "/tmp/notes".to_string(),
            })
            .await
            .expect("建工作区应当成功");
        assert_eq!(workspace.name, "笔记");

        let listed = client.list_workspaces().await.expect("列工作区应当成功");
        assert_eq!(listed.workspaces, vec![workspace.clone()]);
        assert!(
            storage
                .join("workspaces")
                .join(format!("{}.json", workspace.workspace_id))
                .is_file(),
            "服务端应当已经把工作区落到磁盘上"
        );

        let session = client
            .create_session(CreateSessionRequest {
                workspace_id: workspace.workspace_id.clone(),
                local_id: "l-2".into(),
                title: None,
                turns: vec![Turn {
                    turn_id: TurnId::new("t-1"),
                    role: abs_llm::v1::cont::Role::User,
                    text: "离线时记的一句".to_string(),
                    reasoning: String::new(),
                    state: TurnState::Done,
                    tool_calls: Vec::new(),
                    usage: None,
                    notice: None,
                }],
            })
            .await
            .expect("建会话应当成功");
        assert_eq!(session.title, "离线时记的一句", "标题应当由服务端推导");

        let detail = client
            .get_session(workspace.workspace_id.clone(), session.session_id.clone())
            .await
            .expect("读会话应当成功");
        assert_eq!(detail.turns.len(), 1);
        assert_eq!(detail.turns[0].text, "离线时记的一句");

        let sessions_dir = storage
            .join("sessions")
            .join(workspace.workspace_id.as_str());
        assert!(sessions_dir.is_dir(), "会话目录应当已建立");

        client
            .remove_session(workspace.workspace_id.clone(), session.session_id.clone())
            .await
            .expect("删会话应当成功");

        // ── 业务错误：不存在的工作区 ──────────────────────────────────
        let error = client
            .remove_workspace(WorkspaceId::new("w-404"))
            .await
            .expect_err("不存在的工作区应当报错");
        match error {
            RpcError::Business(reply) => assert_eq!(reply.code, ErrorCode::NotFound),
            other => panic!("应当是业务错误，实际: {other:?}"),
        }

        client
            .remove_workspace(workspace.workspace_id.clone())
            .await
            .expect("删工作区应当成功");
        assert!(!sessions_dir.exists(), "删工作区应当级联删掉会话目录");
        assert!(
            client
                .list_workspaces()
                .await
                .expect("列工作区应当成功")
                .workspaces
                .is_empty()
        );

        drop(client);
        server.join().expect("服务端线程应当正常结束");
    }

    /// 测试存储层的错误会被翻成正确的 `ErrorCode`。
    ///
    /// - 手段：直接调用 `store_error_`，喂进三种有代表性的 [`StoreError`]。
    /// - 判断：`NotFound` → `NotFound`、`InvalidId` → `BadRequest`、
    ///   I/O 一类的其它错误 → `Internal`。
    #[test]
    fn store_errors_map_to_protocol_codes_() {
        let not_found = store_error_(StoreError::NotFound {
            kind: "工作区",
            id: "w-1".to_string(),
        });
        assert_eq!(
            not_found.as_business().map(|reply| reply.code),
            Some(ErrorCode::NotFound)
        );

        let invalid = store_error_(StoreError::InvalidId {
            kind: "工作区",
            id: "../evil".to_string(),
        });
        assert_eq!(
            invalid.as_business().map(|reply| reply.code),
            Some(ErrorCode::BadRequest)
        );

        let internal = store_error_(StoreError::BlockingTask("线程没了".to_string()));
        assert_eq!(
            internal.as_business().map(|reply| reply.code),
            Some(ErrorCode::Internal)
        );
    }
}
