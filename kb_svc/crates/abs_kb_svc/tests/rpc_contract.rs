//! 工作区 / 会话两个按域 RPC trait 的**契约测试**。
//!
//! 这个文件同时充当三件事：
//!
//! 1. 证明 [`abs_kb_svc`] 里那套「手写 GAT + `gen_mcf2` 生成的可取消 future」
//!    的形状真的能编译、能跑（形状本身先在
//!    `external/ipc-channel-poc/src/trait_spike.rs` 里验证过）；
//! 2. 给将来两个真实实现（`kb_svc_servo_ipc` 的客户端代理、`kb_core` 的服务端逻辑）
//!    留一份**可直接照抄的样板**：模块级 `async fn` + 宏 + `impl` 里把 GAT 填上；
//! 3. 把"面向 trait 编程"这件事钉住：泛型调用者只认 trait，不知道实现是谁。
//!
//! 需要 nightly 的 `impl_trait_in_assoc_type`——`gen_mcf2` 展开出来的
//! 工厂实现用了 `impl Trait` 作为关联类型。

#![feature(impl_trait_in_assoc_type)]
// `gen_mcf2` 会把 `async fn` 上**显式声明**的生命周期做成生成类型的泛型参数，
// 所以下面那些 `'s` 不能省略——省略之后 clippy 的 `needless_lifetimes` 反而是错的建议。
#![allow(clippy::needless_lifetimes)]

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use abs_cancel::{CancelledToken, NonCancellableToken, TrCancellationToken, TrMayCancel};
use abs_kb_svc::v1::desktop::{
    AddWorkspaceRequest, CreateSessionRequest, ErrorCode, ErrorReply, RpcError, SessionDetail,
    SessionId, SessionList, SessionSummary, TrKbEndpoint, TrSessionService, TrWorkspaceService,
    Workspace, WorkspaceId, WorkspaceList,
};
use gen_mcf2::gen_may_cancel_future;

// ============================================================================
// mock 实现
// ============================================================================

/// mock 的**传输层**错误。
///
/// 真实实现里这里是 ipc-channel 的传输错误；契约测试只需要它证明
/// [`RpcError::Transport`] 能被单独表达。
#[derive(Debug, PartialEq, Eq)]
struct MockTransportError {
    /// 说明。
    reason: String,
}

impl MockTransportError {
    /// 造一个"调用已被取消"的传输错误。
    fn cancelled_() -> Self {
        Self {
            reason: "调用被取消".to_string(),
        }
    }
}

impl core::fmt::Display for MockTransportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl core::error::Error for MockTransportError {}

/// 一个纯内存的 mock `kb_core`。
///
/// 用 `RefCell` 是因为 trait 方法只拿得到 `&self`——真实实现（本地文件存储）
/// 也是同样的形状：共享引用 + 内部可变性。
#[derive(Default)]
struct MockService {
    /// 工作区，按标识索引。
    workspaces_: RefCell<BTreeMap<String, Workspace>>,

    /// 会话，按 `(工作区标识, 会话标识)` 索引。
    sessions_: RefCell<BTreeMap<(String, String), SessionDetail>>,

    /// 标识发号器。
    counter_: Cell<u64>,
}

impl MockService {
    /// 发一个新标识（`w-0` / `s-0` / …）。
    fn next_id_(&self, prefix: &str) -> String {
        let value = self.counter_.get();
        self.counter_.set(value + 1);
        format!("{prefix}-{value}")
    }
}

/// 造一条业务错误。
fn business_(code: ErrorCode, message: impl Into<String>) -> RpcError<MockTransportError> {
    RpcError::Business(ErrorReply {
        code,
        message: message.into(),
    })
}

/// 把一个可取消 future 驱动到完成。
///
/// `gen_mcf2` 生成的是 [`IntoFuture`](std::future::IntoFuture) 而不是
/// [`Future`](std::future::Future)，所以要显式 `into_future()`；
/// 真正的生产者代码里写 `.await` 即可，两者等价。
fn block_on_<F>(future: F) -> F::Output
where
    F: std::future::IntoFuture,
{
    futures_lite::future::block_on(future.into_future())
}

// ── 模块级 async fn：宏只能作用于它们 ──────────────────────────────────

/// mock：列出全部工作区。
#[gen_may_cancel_future(ListWorkspaces)]
async fn list_workspaces_async<'s, C>(
    service: &'s MockService,
    cancel: C,
) -> Result<WorkspaceList, RpcError<MockTransportError>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(RpcError::Transport(MockTransportError::cancelled_()));
    }
    Ok(WorkspaceList {
        workspaces: service.workspaces_.borrow().values().cloned().collect(),
    })
}

/// mock：登记一个工作区，标识由"服务端"分配。
#[gen_may_cancel_future(AddWorkspace)]
async fn add_workspace_async<'s, C>(
    service: &'s MockService,
    request: AddWorkspaceRequest,
    cancel: C,
) -> Result<Workspace, RpcError<MockTransportError>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(RpcError::Transport(MockTransportError::cancelled_()));
    }

    // `request.local_id` 是调用方的簿记字段，服务端不解释它。
    let workspace = Workspace {
        workspace_id: WorkspaceId::new(service.next_id_("w")),
        name: request.name,
        path: request.path,
    };
    service
        .workspaces_
        .borrow_mut()
        .insert(workspace.workspace_id.to_string(), workspace.clone());
    Ok(workspace)
}

/// mock：删除一个工作区，并级联删除它名下的会话。
#[gen_may_cancel_future(RemoveWorkspace)]
async fn remove_workspace_async<'s, C>(
    service: &'s MockService,
    workspace_id: WorkspaceId,
    cancel: C,
) -> Result<(), RpcError<MockTransportError>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(RpcError::Transport(MockTransportError::cancelled_()));
    }

    if service
        .workspaces_
        .borrow_mut()
        .remove(workspace_id.as_str())
        .is_none()
    {
        return Err(business_(
            ErrorCode::NotFound,
            format!("工作区不存在: {workspace_id}"),
        ));
    }
    service
        .sessions_
        .borrow_mut()
        .retain(|(owner, _), _| owner != workspace_id.as_str());
    Ok(())
}

/// mock：列出某个工作区下的会话摘要。
#[gen_may_cancel_future(ListSessions)]
async fn list_sessions_async<'s, C>(
    service: &'s MockService,
    workspace_id: WorkspaceId,
    cancel: C,
) -> Result<SessionList, RpcError<MockTransportError>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(RpcError::Transport(MockTransportError::cancelled_()));
    }

    if !service
        .workspaces_
        .borrow()
        .contains_key(workspace_id.as_str())
    {
        return Err(business_(
            ErrorCode::NotFound,
            format!("工作区不存在: {workspace_id}"),
        ));
    }

    let sessions = service
        .sessions_
        .borrow()
        .iter()
        .filter(|((owner, _), _)| owner == workspace_id.as_str())
        .map(|(_, detail)| detail.summary.clone())
        .collect();
    Ok(SessionList { sessions })
}

/// mock：登记一个会话。
#[gen_may_cancel_future(CreateSession)]
async fn create_session_async<'s, C>(
    service: &'s MockService,
    request: CreateSessionRequest,
    cancel: C,
) -> Result<SessionSummary, RpcError<MockTransportError>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(RpcError::Transport(MockTransportError::cancelled_()));
    }

    if !service
        .workspaces_
        .borrow()
        .contains_key(request.workspace_id.as_str())
    {
        return Err(business_(
            ErrorCode::NotFound,
            format!("工作区不存在: {}", request.workspace_id),
        ));
    }

    let title = request
        .title
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| "新会话".to_string());
    let summary = SessionSummary {
        session_id: SessionId::new(service.next_id_("s")),
        workspace_id: request.workspace_id.clone(),
        title,
        updated_at_millis: 1,
        turn_count: u32::try_from(request.turns.len()).unwrap_or(u32::MAX),
    };
    service.sessions_.borrow_mut().insert(
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

/// mock：读取一个会话的完整内容。
#[gen_may_cancel_future(GetSession)]
async fn get_session_async<'s, C>(
    service: &'s MockService,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    cancel: C,
) -> Result<SessionDetail, RpcError<MockTransportError>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(RpcError::Transport(MockTransportError::cancelled_()));
    }

    service
        .sessions_
        .borrow()
        .get(&(workspace_id.to_string(), session_id.to_string()))
        .cloned()
        .ok_or_else(|| {
            business_(
                ErrorCode::NotFound,
                format!("会话不存在: {session_id}（工作区 {workspace_id}）"),
            )
        })
}

/// mock：删除一个会话。
#[gen_may_cancel_future(RemoveSession)]
async fn remove_session_async<'s, C>(
    service: &'s MockService,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    cancel: C,
) -> Result<(), RpcError<MockTransportError>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(RpcError::Transport(MockTransportError::cancelled_()));
    }

    if service
        .sessions_
        .borrow_mut()
        .remove(&(workspace_id.to_string(), session_id.to_string()))
        .is_none()
    {
        return Err(business_(
            ErrorCode::NotFound,
            format!("会话不存在: {session_id}"),
        ));
    }
    Ok(())
}

// ── 把生成的 future 填进 trait 的关联类型 ─────────────────────────────

impl TrKbEndpoint for MockService {
    type Error = MockTransportError;
}

impl TrWorkspaceService for MockService {
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

impl TrSessionService for MockService {
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
// 测试
// ============================================================================

/// 造一个工作区登记请求。
fn add_request_(name: &str, path: &str) -> AddWorkspaceRequest {
    AddWorkspaceRequest {
        local_id: format!("l-{name}").into(),
        name: name.to_string(),
        path: path.to_string(),
    }
}

/// 测试工作区的增、查、删走完一轮。
///
/// - 手段：对一个全新的 `MockService` 依次调用 `add_workspace`、`list_workspaces`、
///   `remove_workspace`、再 `list_workspaces`，每一步都 `.await`（不可取消路径）。
/// - 判断：新增返回的工作区带着服务端分配的标识；列表长度依次是 1、0；
///   删除后再列出来是空的——即 trait 的读写在同一个实现里自洽。
#[test]
fn workspace_crud_round_trip_() {
    let service = MockService::default();

    let added = block_on_(service.add_workspace(add_request_("笔记", "/tmp/notes")))
        .expect("登记工作区应当成功");
    assert_eq!(added.name, "笔记");
    assert!(added.workspace_id.as_str().starts_with("w-"));

    let listed = block_on_(service.list_workspaces()).expect("应当能列出");
    assert_eq!(listed.workspaces, vec![added.clone()]);

    block_on_(service.remove_workspace(added.workspace_id.clone())).expect("删除应当成功");

    let listed = block_on_(service.list_workspaces()).expect("应当能列出");
    assert!(listed.workspaces.is_empty());
}

/// 测试目标不存在时返回的是**业务错误**而不是传输错误。
///
/// - 手段：对一个空的服务调用 `remove_workspace`。
/// - 判断：`RpcError::Business`，其 `code` 是 `NotFound`，且 `is_transport()` 为假——
///   这两类失败必须能被调用方区分开（`abs_kb_svc/README.md` §5 第 7 条）。
#[test]
fn missing_workspace_is_business_error_() {
    let service = MockService::default();
    let error = block_on_(service.remove_workspace(WorkspaceId::new("w-404")))
        .expect_err("不存在的工作区应当报错");

    assert!(!error.is_transport(), "应当是业务错误，实际: {error:?}");
    let business = error.as_business().expect("应当是业务错误");
    assert_eq!(business.code, ErrorCode::NotFound);
}

/// 测试会话的增、查（列表 + 详情）、删走完一轮。
///
/// - 手段：先登记工作区，再 `create_session`、`list_sessions`、`get_session`、
///   `remove_session`。
/// - 判断：会话摘要带服务端分配的标识与工作区归属；列表只回摘要；
///   详情里的 `turns` 与创建时提交的一致（客户端离线攒下的历史不会丢）。
#[test]
fn session_crud_round_trip_() {
    let service = MockService::default();
    let workspace = block_on_(service.add_workspace(add_request_("笔记", "/tmp/notes")))
        .expect("登记工作区应当成功");

    let turn = abs_kb_svc::v1::desktop::Turn {
        turn_id: "t-1".into(),
        role: abs_llm::v1::cont::Role::User,
        text: "离线时记的一句".to_string(),
        reasoning: String::new(),
        state: abs_kb_svc::v1::desktop::TurnState::Done,
        tool_calls: Vec::new(),
        usage: None,
        notice: None,
    };

    let created = block_on_(service.create_session(CreateSessionRequest {
        workspace_id: workspace.workspace_id.clone(),
        local_id: "l-1".into(),
        title: None,
        turns: vec![turn.clone()],
    }))
    .expect("登记会话应当成功");
    assert_eq!(created.title, "新会话", "缺省标题应当由服务端补齐");
    assert_eq!(created.workspace_id, workspace.workspace_id);
    assert!(created.session_id.as_str().starts_with("s-"));

    let listed =
        block_on_(service.list_sessions(workspace.workspace_id.clone())).expect("应当能列出会话");
    assert_eq!(listed.sessions, vec![created.clone()]);

    let detail =
        block_on_(service.get_session(workspace.workspace_id.clone(), created.session_id.clone()))
            .expect("应当能读到会话");
    assert_eq!(detail.summary, created);
    assert_eq!(detail.turns, vec![turn]);

    block_on_(service.remove_session(workspace.workspace_id.clone(), created.session_id.clone()))
        .expect("删除会话应当成功");

    let error =
        block_on_(service.get_session(workspace.workspace_id.clone(), created.session_id.clone()))
            .expect_err("删掉的会话不应还能读到");
    assert_eq!(
        error.as_business().map(|business| business.code),
        Some(ErrorCode::NotFound)
    );
}

/// 测试删除工作区会级联删掉它的会话。
///
/// - 手段：登记工作区 → 建会话 → 删工作区 → 直接问会话是否还在
///   （绕过 `list_sessions` 对工作区存在性的检查）。
/// - 判断：删工作区之后 `get_session` 报 `NotFound`——级联确实发生了。
#[test]
fn removing_workspace_cascades_sessions_() {
    let service = MockService::default();
    let workspace = block_on_(service.add_workspace(add_request_("笔记", "/tmp/notes")))
        .expect("登记工作区应当成功");
    let session = block_on_(service.create_session(CreateSessionRequest {
        workspace_id: workspace.workspace_id.clone(),
        local_id: "l-1".into(),
        title: Some("第一问".to_string()),
        turns: Vec::new(),
    }))
    .expect("登记会话应当成功");

    block_on_(service.remove_workspace(workspace.workspace_id.clone()))
        .expect("删除工作区应当成功");

    let error =
        block_on_(service.get_session(workspace.workspace_id.clone(), session.session_id.clone()))
            .expect_err("工作区没了，会话也不该还在");
    assert_eq!(
        error.as_business().map(|business| business.code),
        Some(ErrorCode::NotFound)
    );
}

/// 测试可取消路径：用已经取消的令牌调用会落到 `RpcError::Transport`。
///
/// - 手段：用 `CancelledToken::new()` 走 `may_cancel_with`。
/// - 判断：返回 `RpcError::Transport`（实现方的错误类型），而不是业务错误——
///   这正是 `rpc_` 模块文档里"取消归到传输侧"那条约定的体现；
///   同时用 `NonCancellableToken` 走一遍，确认同一份实现在两条路径下都可用。
#[test]
fn cancellation_lands_on_transport_error_() {
    let service = MockService::default();

    let outcome = block_on_(
        service
            .list_workspaces()
            .may_cancel_with(CancelledToken::new()),
    );
    assert!(outcome.is_err(), "已取消的调用不该成功");

    let outcome = block_on_(
        service
            .list_workspaces()
            .may_cancel_with(NonCancellableToken::new()),
    );
    assert!(outcome.is_ok(), "不可取消的令牌应当照常完成");
}

/// 测试"面向 trait 编程"的泛型调用者不需要知道实现是谁。
///
/// - 手段：写一个只受 `TrWorkspaceService + TrSessionService` 约束的泛型函数，
///   把 mock 实现传进去，让它跑完"建工作区 → 建会话 → 读会话"。
/// - 判断：泛型函数能编译并得到正确结果——这条约束正是 `kb_svc_servo_ipc`
///   的客户端代理与 `kb_core` 的服务端逻辑要共同满足的东西。
#[test]
fn generic_caller_only_knows_the_traits_() {
    /// 只认 trait 的调用者。
    async fn run_<S>(service: &S) -> Result<SessionDetail, RpcError<S::Error>>
    where
        S: TrWorkspaceService + TrSessionService,
    {
        let workspace = service
            .add_workspace(AddWorkspaceRequest {
                local_id: "l-1".into(),
                name: "笔记".to_string(),
                path: "/tmp/notes".to_string(),
            })
            .await?;
        let session = service
            .create_session(CreateSessionRequest {
                workspace_id: workspace.workspace_id.clone(),
                local_id: "l-2".into(),
                title: Some("泛型调用".to_string()),
                turns: Vec::new(),
            })
            .await?;
        service
            .get_session(workspace.workspace_id, session.session_id)
            .await
    }

    let service = MockService::default();
    let detail = block_on_(run_(&service)).expect("泛型调用应当成功");
    assert_eq!(detail.summary.title, "泛型调用");
    assert!(detail.turns.is_empty());
}
