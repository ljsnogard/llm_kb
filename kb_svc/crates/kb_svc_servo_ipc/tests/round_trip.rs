//! `kb_svc_servo_ipc` 的**端到端**测试：真起一个服务端、真按 ipc-channel 走线。
//!
//! 与 `abs_kb_svc/tests/rpc_contract.rs` 的分工：
//!
//! | 文件 | 验证什么 |
//! | :--- | :--- |
//! | `rpc_contract.rs`（在 `abs_kb_svc`） | trait 的形状对不对、mock 直接调用语义对不对 |
//! | 本文件 | **跨进程通道**这一段：引导、三条通道、请求/应答、业务错误、取消、多客户端 |
//!
//! 服务端与客户端在同一个进程里（不同线程），但走的是真实的 ipc-channel
//! socketpair 与文件描述符传递——与跨进程路径完全一致。
//!
//! 需要 nightly 的 `impl_trait_in_assoc_type`：业务实现用 `gen_mcf2` 展开。

#![feature(impl_trait_in_assoc_type)]
// `gen_mcf2` 会把 `async fn` 上显式声明的生命周期做成生成类型的泛型参数。
#![allow(clippy::needless_lifetimes)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use abs_cancel::{CancelledToken, TrCancellationToken, TrMayCancel};
use abs_kb_svc::v1::desktop::{
    AddWorkspaceRequest, CreateSessionRequest, ErrorCode, ErrorReply, RpcError, SessionDetail,
    SessionId, SessionList, SessionSummary, TrKbEndpoint, TrSessionService, TrWorkspaceService,
    Workspace, WorkspaceId, WorkspaceList,
};
use gen_mcf2::gen_may_cancel_future;
use kb_svc_servo_ipc::{Client, Listener, ServoIpcError};

// ============================================================================
// 一个最小的内存版 kb_core 业务实现
// ============================================================================

/// 内存结构。
#[derive(Default)]
struct Inner {
    /// 工作区。
    workspaces_: BTreeMap<String, Workspace>,

    /// 会话。
    sessions_: BTreeMap<(String, String), SessionDetail>,

    /// 标识发号器。
    counter_: u64,
}

/// 测试用的业务实现。
///
/// 真实实现（`kb_core`）把同样的方法转发到本地文件存储；
/// 这里只求"够用且能看出请求真的走到了服务端"。
#[derive(Default)]
struct TestService {
    /// 内部状态。
    inner_: Mutex<Inner>,
}

impl TestService {
    /// 发一个新标识。
    fn next_id_(&self, prefix: &str) -> String {
        let mut inner = lock_(&self.inner_);
        inner.counter_ += 1;
        format!("{prefix}-{}", inner.counter_)
    }
}

/// 造一条业务错误应答。
fn business_<T, E>(code: ErrorCode, message: impl Into<String>) -> Result<T, RpcError<E>> {
    Err(RpcError::Business(ErrorReply {
        code,
        message: message.into(),
    }))
}

// ── 模块级 async fn：宏只能作用于它们 ──────────────────────────────────

/// 服务端：列出工作区。
#[gen_may_cancel_future(ListWorkspaces)]
async fn list_workspaces_async<'s, C>(
    service: &'s TestService,
    cancel: C,
) -> Result<WorkspaceList, RpcError<ServoIpcError>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(RpcError::Transport(ServoIpcError::Cancelled));
    }
    Ok(WorkspaceList {
        workspaces: lock_(&service.inner_)
            .workspaces_
            .values()
            .cloned()
            .collect(),
    })
}

/// 服务端：登记工作区。
#[gen_may_cancel_future(AddWorkspace)]
async fn add_workspace_async<'s, C>(
    service: &'s TestService,
    request: AddWorkspaceRequest,
    cancel: C,
) -> Result<Workspace, RpcError<ServoIpcError>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(RpcError::Transport(ServoIpcError::Cancelled));
    }
    let workspace = Workspace {
        workspace_id: WorkspaceId::new(service.next_id_("w")),
        name: request.name,
        path: request.path,
    };
    lock_(&service.inner_)
        .workspaces_
        .insert(workspace.workspace_id.to_string(), workspace.clone());
    Ok(workspace)
}

/// 服务端：删除工作区（级联删会话）。
#[gen_may_cancel_future(RemoveWorkspace)]
async fn remove_workspace_async<'s, C>(
    service: &'s TestService,
    workspace_id: WorkspaceId,
    cancel: C,
) -> Result<(), RpcError<ServoIpcError>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(RpcError::Transport(ServoIpcError::Cancelled));
    }
    let mut inner = lock_(&service.inner_);
    if inner.workspaces_.remove(workspace_id.as_str()).is_none() {
        drop(inner);
        return business_(ErrorCode::NotFound, format!("工作区不存在: {workspace_id}"));
    }
    inner
        .sessions_
        .retain(|(owner, _), _| owner != workspace_id.as_str());
    Ok(())
}

/// 服务端：列出会话。
#[gen_may_cancel_future(ListSessions)]
async fn list_sessions_async<'s, C>(
    service: &'s TestService,
    workspace_id: WorkspaceId,
    cancel: C,
) -> Result<SessionList, RpcError<ServoIpcError>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(RpcError::Transport(ServoIpcError::Cancelled));
    }
    let inner = lock_(&service.inner_);
    if !inner.workspaces_.contains_key(workspace_id.as_str()) {
        drop(inner);
        return business_(ErrorCode::NotFound, format!("工作区不存在: {workspace_id}"));
    }
    Ok(SessionList {
        sessions: inner
            .sessions_
            .iter()
            .filter(|((owner, _), _)| owner == workspace_id.as_str())
            .map(|(_, detail)| detail.summary.clone())
            .collect(),
    })
}

/// 服务端：登记会话。
#[gen_may_cancel_future(CreateSession)]
async fn create_session_async<'s, C>(
    service: &'s TestService,
    request: CreateSessionRequest,
    cancel: C,
) -> Result<SessionSummary, RpcError<ServoIpcError>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(RpcError::Transport(ServoIpcError::Cancelled));
    }
    if !lock_(&service.inner_)
        .workspaces_
        .contains_key(request.workspace_id.as_str())
    {
        return business_(
            ErrorCode::NotFound,
            format!("工作区不存在: {}", request.workspace_id),
        );
    }
    let summary = SessionSummary {
        session_id: SessionId::new(service.next_id_("s")),
        workspace_id: request.workspace_id.clone(),
        title: request
            .title
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| "新会话".to_string()),
        updated_at_millis: 1,
        turn_count: u32::try_from(request.turns.len()).unwrap_or(u32::MAX),
    };
    lock_(&service.inner_).sessions_.insert(
        (
            summary.workspace_id.to_string(),
            summary.session_id.to_string(),
        ),
        SessionDetail {
            summary: summary.clone(),
            turns: request.turns,
        },
    );
    Ok(summary)
}

/// 服务端：读会话。
#[gen_may_cancel_future(GetSession)]
async fn get_session_async<'s, C>(
    service: &'s TestService,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    cancel: C,
) -> Result<SessionDetail, RpcError<ServoIpcError>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(RpcError::Transport(ServoIpcError::Cancelled));
    }
    match lock_(&service.inner_)
        .sessions_
        .get(&(workspace_id.to_string(), session_id.to_string()))
        .cloned()
    {
        Some(detail) => Ok(detail),
        None => business_(
            ErrorCode::NotFound,
            format!("会话不存在: {session_id}（工作区 {workspace_id}）"),
        ),
    }
}

/// 服务端：删会话。
#[gen_may_cancel_future(RemoveSession)]
async fn remove_session_async<'s, C>(
    service: &'s TestService,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    cancel: C,
) -> Result<(), RpcError<ServoIpcError>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(RpcError::Transport(ServoIpcError::Cancelled));
    }
    if lock_(&service.inner_)
        .sessions_
        .remove(&(workspace_id.to_string(), session_id.to_string()))
        .is_none()
    {
        return business_(ErrorCode::NotFound, format!("会话不存在: {session_id}"));
    }
    Ok(())
}

impl TrKbEndpoint for TestService {
    type Error = ServoIpcError;
}

impl TrWorkspaceService for TestService {
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

impl TrSessionService for TestService {
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

// ============================================================================
// 测试脚手架
// ============================================================================

/// 驱动一个可取消 future 到完成。
fn block_on_<F>(future: F) -> F::Output
where
    F: std::future::IntoFuture,
{
    futures_lite::future::block_on(future.into_future())
}

/// 取锁，忽略中毒。
fn lock_<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 造一个临时的 runtime 目录。
fn temp_runtime_() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("应当能创建临时目录");
    let runtime = dir.path().join("run");
    (dir, runtime)
}

/// 在一条**独立线程**上起服务端，连续服务 `clients` 个客户端。
///
/// `accept()` 与 `serve()` 里的阻塞部分都在那条线程上——这正是 `kb_core`
/// 将来要用 `compio::runtime::spawn_blocking` 做的事。
///
/// 业务实现只建一次：多个客户端看到的是同一份状态，这才像"一个服务端进程"。
fn spawn_server_(runtime: PathBuf, clients: usize) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let listener = Listener::bind(&runtime).expect("应当能绑定端点");
        let service = TestService::default();
        for index in 0..clients {
            let connection = listener.accept().expect("应当能接受连接");
            futures_lite::future::block_on(connection.serve(&service)).expect("serve 不应当失败");
            println!("服务端：第 {index} 个客户端已断开");
        }
    })
}

/// 造一个"本地先创建"的工作区登记请求。
fn add_request_(name: &str) -> AddWorkspaceRequest {
    AddWorkspaceRequest {
        local_id: format!("l-{name}").into(),
        name: name.to_string(),
        path: format!("/tmp/{name}"),
    }
}

// ============================================================================
// 测试
// ============================================================================

/// 测试工作区与会话的增删查改能整条走通 ipc-channel。
///
/// - 手段：一条独立线程上起服务端（`Listener` + `Connection::serve`），
///   测试线程用 `Client::connect` 连上，依次做
///   「建工作区 → 列工作区 → 建会话 → 列会话 → 读会话 → 删会话 → 删工作区 → 再列」。
/// - 判断：每一步的业务载荷都正确（标识由服务端分配、会话标题由服务端补齐、
///   详情里的消息与创建时一致）；最后工作区列表为空——请求真的到了另一侧，
///   应答也真的回来了。
#[test]
fn round_trip_covers_workspace_and_session_crud_() {
    let (_guard, runtime) = temp_runtime_();
    let server = spawn_server_(runtime.clone(), 1);

    let client = Client::connect(&runtime).expect("应当能连上服务端");

    let workspace =
        block_on_(client.add_workspace(add_request_("笔记"))).expect("建工作区应当成功");
    assert_eq!(workspace.name, "笔记");
    assert_eq!(workspace.path, "/tmp/笔记");
    assert!(workspace.workspace_id.as_str().starts_with("w-"));

    let listed = block_on_(client.list_workspaces()).expect("列工作区应当成功");
    assert_eq!(listed.workspaces, vec![workspace.clone()]);

    let session = block_on_(client.create_session(CreateSessionRequest {
        workspace_id: workspace.workspace_id.clone(),
        local_id: "l-session-1".into(),
        title: None,
        turns: Vec::new(),
    }))
    .expect("建会话应当成功");
    assert_eq!(session.title, "新会话", "缺省标题应当由服务端补齐");
    assert_eq!(session.workspace_id, workspace.workspace_id);
    assert!(session.session_id.as_str().starts_with("s-"));

    let sessions =
        block_on_(client.list_sessions(workspace.workspace_id.clone())).expect("列会话应当成功");
    assert_eq!(sessions.sessions, vec![session.clone()]);

    let detail =
        block_on_(client.get_session(workspace.workspace_id.clone(), session.session_id.clone()))
            .expect("读会话应当成功");
    assert_eq!(detail.summary, session);

    block_on_(client.remove_session(workspace.workspace_id.clone(), session.session_id.clone()))
        .expect("删会话应当成功");

    block_on_(client.remove_workspace(workspace.workspace_id.clone())).expect("删工作区应当成功");
    let listed = block_on_(client.list_workspaces()).expect("再列工作区应当成功");
    assert!(listed.workspaces.is_empty());

    drop(client);
    server.join().expect("服务端线程应当正常结束");
}

/// 测试业务错误能原样穿过通道，不被误当成传输失败。
///
/// - 手段：对一个不存在的工作区调用 `remove_workspace`。
/// - 判断：拿到的是 `RpcError::Business`，`code` 为 `NotFound`；
///   这条错误是服务端**正常应答**的一个分支，不是 `Transport`。
#[test]
fn business_errors_survive_the_wire_() {
    let (_guard, runtime) = temp_runtime_();
    let server = spawn_server_(runtime.clone(), 1);
    let client = Client::connect(&runtime).expect("应当能连上服务端");

    let error = block_on_(client.remove_workspace(WorkspaceId::new("w-404")))
        .expect_err("不存在的工作区应当报错");

    match error {
        RpcError::Business(reply) => {
            assert_eq!(reply.code, ErrorCode::NotFound);
            assert!(reply.message.contains("w-404"), "实际: {}", reply.message);
        }
        other => panic!("应当是业务错误，实际: {other:?}"),
    }

    drop(client);
    server.join().expect("服务端线程应当正常结束");
}

/// 测试客户端会在服务端还没公布端点时重试，而不是立刻失败。
///
/// - 手段：先起一个"睡 150 ms 再绑定并 accept"的服务端线程，然后立刻
///   `Client::connect`（给足超时）。
/// - 判断：连接最终成功且一次调用能完成——重试路径确实在工作，
///   这也正是多次 `accept` 之间名字文件短暂缺失时客户端要走的路径。
#[test]
fn client_retries_until_the_server_publishes_() {
    let (_guard, runtime) = temp_runtime_();
    let delayed_runtime = runtime.clone();
    let server = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        let listener = Listener::bind(&delayed_runtime).expect("应当能绑定端点");
        let connection = listener.accept().expect("应当能接受连接");
        futures_lite::future::block_on(connection.serve(&TestService::default()))
            .expect("serve 不应当失败");
    });

    let client = Client::connect_with_timeout(&runtime, std::time::Duration::from_secs(5))
        .expect("重试之后应当能连上");
    let listed = block_on_(client.list_workspaces()).expect("调用应当成功");
    assert!(listed.workspaces.is_empty());

    drop(client);
    server.join().expect("服务端线程应当正常结束");
}

/// 测试一个服务端能接着服务第二个客户端（one-shot 端点重建 + 名字重发），
/// 并且两个客户端看到的是**同一份服务端状态**。
///
/// - 手段：服务端线程连续 `accept` 两次、共享同一个业务实现；测试里先连第一个
///   客户端、建一个工作区、丢弃它，再连第二个客户端、列工作区。
/// - 判断：第二个客户端看得见第一个客户端建的工作区——既证明"端点重建 + 名字
///   重发 + 客户端重试"这条绕法在真实代码里生效（`IpcOneShotServer` 只能接受
///   一次连接），也证明服务端状态没有随连接一起丢。
#[test]
fn a_second_client_is_served_after_the_first_disconnects_() {
    let (_guard, runtime) = temp_runtime_();
    let server = spawn_server_(runtime.clone(), 2);

    {
        let client = Client::connect(&runtime).expect("第一个客户端应当能连上");
        block_on_(client.add_workspace(add_request_("第一个"))).expect("第一次调用应当成功");
    }

    {
        let client = Client::connect(&runtime).expect("第二个客户端应当能连上");
        let listed = block_on_(client.list_workspaces()).expect("第二次调用应当成功");
        assert_eq!(listed.workspaces.len(), 1, "第二个客户端应当看到同一份状态");
        assert_eq!(listed.workspaces[0].name, "第一个");
    }

    server.join().expect("服务端线程应当正常结束");
}

/// 测试已经取消的调用会立刻收手，不会挂住调用方。
///
/// - 手段：用 `CancelledToken` 走 `.may_cancel_with(..)`。
/// - 判断：立刻拿到 `RpcError::Transport(ServoIpcError::Cancelled)`——
///   对应 `rpc_` 模块文档里"取消归到传输侧"那条约定。
#[test]
fn cancelled_call_returns_instead_of_hanging_() {
    let (_guard, runtime) = temp_runtime_();
    let server = spawn_server_(runtime.clone(), 1);
    let client = Client::connect(&runtime).expect("应当能连上服务端");

    let outcome = block_on_(
        client
            .list_workspaces()
            .may_cancel_with(CancelledToken::new()),
    );
    match outcome {
        Err(RpcError::Transport(ServoIpcError::Cancelled)) => {}
        other => panic!("应当是已被取消的传输错误，实际: {other:?}"),
    }

    drop(client);
    server.join().expect("服务端线程应当正常结束");
}

/// 测试连不上的时候会在超时后明确报错，而不是永远阻塞。
///
/// - 手段：对一个没有任何服务端的空目录调用 `connect_with_timeout(100ms)`。
/// - 判断：返回 `ServoIpcError::ConnectTimeout`。
#[test]
fn connect_times_out_without_a_server_() {
    let (_guard, runtime) = temp_runtime_();
    let error = Client::connect_with_timeout(&runtime, std::time::Duration::from_millis(100))
        .expect_err("没有服务端时应当超时");

    assert!(
        matches!(error, ServoIpcError::ConnectTimeout { .. }),
        "实际错误: {error}"
    );
}
