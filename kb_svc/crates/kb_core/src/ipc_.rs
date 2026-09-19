//! 把本地文件存储接上 IPC：`kb_core` 这一侧的按域 RPC trait 实现。
//!
//! [`KbService`] 把 [`TrHandshake`] / [`TrWorkspaceService`] / [`TrSessionService`]
//! 的每个方法原样转发给 [`Store`]，并把 [`StoreError`] 翻成协议里的业务错误：
//!
//! | `StoreError` | `ErrorCode` |
//! | :--- | :--- |
//! | `NotFound` | `NotFound` |
//! | `InvalidId` | `BadRequest` |
//! | `Cancelled` | `Internal`（服务端侧被取消） |
//! | 其它（I/O、解码、阻塞任务） | `Internal` |
//!
//! # 生成域：临时模拟的 LLM
//!
//! [`TrGeneration::ask`] 还没有真正的 LLM 插件可接，因此实现成
//! **"把问题按字符逆序输出"**：一次提问落两条消息（`user` + 逆序的
//! `assistant`），应答是提问之后的完整会话。细节与替换路径见 [`ask_async`]。
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
    AddWorkspaceRequest, AskRequest, ClientInfo, CreateSessionRequest, ErrorCode, ErrorReply,
    Notice, PROTOCOL_VERSION, RpcError, ServerInfo, SessionDetail, SessionId, SessionList,
    SessionSummary, TrGeneration, TrHandshake, TrKbEndpoint, TrSessionService, TrWorkspaceService,
    Turn, TurnId, TurnState, Workspace, WorkspaceId, WorkspaceList,
};
use abs_llm::v1::cont::Role;
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
        // 调用方输入不合法（空名字、空的「新会话」）：业务拒绝，不是服务端故障。
        StoreError::EmptySession | StoreError::EmptyName { .. } => ErrorCode::BadRequest,
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

/// [`TrWorkspaceService::rename_workspace`] 的服务端实现。
#[gen_may_cancel_future(RenameWorkspace, pub)]
pub async fn rename_workspace_async<'s, C>(
    service: &'s KbService,
    workspace_id: WorkspaceId,
    name: String,
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
        .rename_workspace(&workspace_id, &name)
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

/// [`TrGeneration::ask`] 的服务端实现：**临时模拟的 LLM**。
///
/// 真正的 LLM 插件还没接，所以这里把所有提问都当成"让模型复述"：
///
/// 1. 把客户端给的那条问题原样记为 `user` 回合（`turn_id` 用客户端生成的那个）；
/// 2. 生成一条 `assistant` 回合，正文是**问题按字符逆序**的结果，并挂一条说明性
///    [`Notice`]，让界面一眼看出这是模拟而不是真模型；
/// 3. 用 [`Store::append_turns`] 落盘（摘要里的 `turn_count` 与
///    `updated_at_millis` 由存储层维护）；
/// 4. 回**提问之后**的完整会话。
///
/// # 幂等（草稿流程依赖它）
///
/// 客户端的「新会话」是一个**草稿**：首次提问时先发 `CreateSession`（把这个
/// 问题作为首条 `user` 消息，好让 `kb_core` 据此起名），紧接着再发本条 `Ask`。
/// 于是 `Ask` 到这里时，用户回合**已经存在**了。
///
/// 所以这里按 `turn_id` 去重：
///
/// - 已经有这一轮的用户回合 → 不再重复添加；
/// - 已经有这一轮的助手回合（标识由用户回合确定性推导，见 [`answer_turn_id_`]）
///   → 原样回会话，不再生成第二条回答。
///
/// 这让"先建会话、再提问"与"重试一次提问"都安全。
///
/// 走"同步落盘再回详情"是为了先验证"新增会话内容在下次连线依然可见"这条目标；
/// 换成真 LLM 时，这里会变成事件流（见 [`TrGeneration`] 的文档），
/// [`Store::append_turns`] 仍会是落盘入口。
#[gen_may_cancel_future(Ask, pub)]
pub async fn ask_async<'s, C>(
    service: &'s KbService,
    request: AskRequest,
    cancel: C,
) -> Result<SessionDetail, RpcError<Infallible>>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(cancelled_());
    }

    let workspace_id = request.workspace_id.clone();
    let session_id = request.session_id.clone();

    let existing = service
        .store_
        .get_session(&workspace_id, &session_id)
        .may_cancel_with(cancel.child_token())
        .await
        .map_err(store_error_)?;

    let answer_id = answer_turn_id_(&request.turn_id);
    let has_user = existing
        .turns
        .iter()
        .any(|turn| turn.turn_id == request.turn_id);
    let has_answer = existing
        .turns
        .iter()
        .any(|turn| turn.turn_id == answer_id);

    if has_answer {
        return Ok(existing);
    }

    let mut turns = Vec::with_capacity(if has_user { 1 } else { 2 });
    if !has_user {
        turns.push(user_turn_(&request));
    }
    turns.push(assistant_turn_(&request, answer_id));

    service
        .store_
        .append_turns(&workspace_id, &session_id, &turns)
        .may_cancel_with(cancel.child_token())
        .await
        .map_err(store_error_)?;

    service
        .store_
        .get_session(&workspace_id, &session_id)
        .may_cancel_with(cancel)
        .await
        .map_err(store_error_)
}

/// 一次提问对应的用户回合。
fn user_turn_(request: &AskRequest) -> Turn {
    Turn {
        turn_id: request.turn_id.clone(),
        role: Role::User,
        text: request.question.clone(),
        reasoning: String::new(),
        state: TurnState::Done,
        tool_calls: Vec::new(),
        usage: None,
        notice: None,
    }
}

/// 一次提问对应的助手回合标识：**由用户回合标识确定性推导**。
///
/// 不用随机标识，是为了让 [`ask_async`] 的"已经有这一轮回答"判断成立——
/// 否则重试就会多出一条回答。
fn answer_turn_id_(user_turn_id: &TurnId) -> TurnId {
    TurnId::new(format!("{}-a", user_turn_id.as_str()))
}

/// 一次提问对应的助手回合（临时模拟的 LLM：正文是问题的逆序）。
fn assistant_turn_(request: &AskRequest, turn_id: TurnId) -> Turn {
    Turn {
        turn_id,
        role: Role::Assistant,
        text: simulated_answer_(&request.question),
        reasoning: String::new(),
        state: TurnState::Done,
        tool_calls: Vec::new(),
        usage: None,
        notice: Some(Notice {
            message: "（kb_core 临时模拟的 LLM：回答是把问题逆序输出）".to_string(),
            is_error: false,
        }),
    }
}

/// 临时模拟的 LLM：把问题按**字符**（不是字节）逆序输出，中文不会碎成半个。
fn simulated_answer_(question: &str) -> String {
    question.chars().rev().collect()
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

/// [`TrSessionService::rename_session`] 的服务端实现。
///
/// `title` 只有空白时，存储层会回去从首条用户消息推导（见
/// [`Store::rename_session`]），所以"清掉手工起的名字"也是一次普通调用。
#[gen_may_cancel_future(RenameSession, pub)]
pub async fn rename_session_async<'s, C>(
    service: &'s KbService,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    title: String,
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
        .rename_session(&workspace_id, &session_id, &title)
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
    type RenameWorkspace<'f>
        = RenameWorkspaceAsync<'f, 'f>
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

    fn rename_workspace<'f>(
        &'f self,
        workspace_id: WorkspaceId,
        name: String,
    ) -> Self::RenameWorkspace<'f> {
        RenameWorkspaceAsync::new(self, workspace_id, name)
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
    type RenameSession<'f>
        = RenameSessionAsync<'f, 'f>
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

    fn rename_session<'f>(
        &'f self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
        title: String,
    ) -> Self::RenameSession<'f> {
        RenameSessionAsync::new(self, workspace_id, session_id, title)
    }
}

impl TrGeneration for KbService {
    type Ask<'f>
        = AskAsync<'f, 'f>
    where
        Self: 'f;

    fn ask<'f>(&'f self, request: AskRequest) -> Self::Ask<'f> {
        AskAsync::new(self, request)
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

    /// 向一个会话提一次问（测试用的小助手）。
    async fn ask_(
        service: &KbService,
        workspace_id: &WorkspaceId,
        session_id: &SessionId,
        turn_id: &str,
        question: &str,
    ) -> SessionDetail {
        service
            .ask(AskRequest {
                workspace_id: workspace_id.clone(),
                session_id: session_id.clone(),
                turn_id: TurnId::new(turn_id),
                question: question.to_string(),
                service_id: None,
            })
            .await
            .expect("提问应当成功")
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

    /// 测试模拟 LLM 的三小块：逆序、助手回合标识的确定性、用户回合的构造。
    ///
    /// - 手段：用一个中文问题分别调用 [`simulated_answer_`]、[`answer_turn_id_`]
    ///   与 [`user_turn_`]。
    /// - 判断：回答按**字符**逆序（中文按字而不是按字节倒过来）；助手回合标识由
    ///   用户回合标识确定性推导（同一个用户回合永远得到同一个回答标识，幂等靠它）；
    ///   用户回合保留客户端给的 `turn_id` 与问题原文。
    #[test]
    fn simulated_answer_reverses_by_chars_() {
        assert_eq!(simulated_answer_("abc你好"), "好你cba");

        let user_id = TurnId::new("t-1");
        assert_eq!(answer_turn_id_(&user_id), TurnId::new("t-1-a"));
        assert_eq!(
            answer_turn_id_(&user_id),
            answer_turn_id_(&user_id),
            "同一个用户回合应当推导出同一个回答标识"
        );

        let request = AskRequest {
            workspace_id: WorkspaceId::new("w-1"),
            session_id: SessionId::new("s-1"),
            turn_id: user_id.clone(),
            question: "abc你好".to_string(),
            service_id: None,
        };
        let user = user_turn_(&request);
        assert_eq!(user.role, Role::User);
        assert_eq!(user.turn_id, user_id);
        assert_eq!(user.text, "abc你好");

        let assistant = assistant_turn_(&request, answer_turn_id_(&request.turn_id));
        assert_eq!(assistant.role, Role::Assistant);
        assert_eq!(assistant.text, "好你cba");
        assert_eq!(assistant.state, TurnState::Done);
        let notice = assistant.notice.as_ref().expect("助手回合应当带说明");
        assert!(!notice.is_error, "说明不是错误: {}", notice.message);
    }

    /// 测试 `Ask` 把一问一答落盘，且重开存储仍能看到（"下次连线依然可见"）。
    ///
    /// - 手段：在临时目录上建工作区与会话，直接对 [`KbService::ask`] 提问
    ///   （中文问题），然后用同一个存储根目录重新 `Store::open` 读回。
    /// - 判断：应答里的会话有两回合，正文分别是问题与它的逆序；`turn_count` 为 2；
    ///   重新打开存储后两条消息仍然在——落盘的是文件，不是进程内缓存。
    #[compio::test]
    async fn ask_persists_the_exchange_for_the_next_connection_() {
        let (_guard, storage, _runtime) = temp_dirs_();
        let store = Store::open(&storage).await.expect("应当能打开存储");
        let workspace = store
            .add_workspace("笔记", "/tmp/notes")
            .await
            .expect("应当能新增工作区");
        let session = store
            .create_session(
                &workspace.workspace_id,
                Some("第一问".to_string()),
                Vec::new(),
            )
            .await
            .expect("应当能新建会话");
        let service = KbService::new(store);

        let detail = service
            .ask(AskRequest {
                workspace_id: workspace.workspace_id.clone(),
                session_id: session.session_id.clone(),
                turn_id: TurnId::new("t-1"),
                question: "你好世界".to_string(),
                service_id: None,
            })
            .await
            .expect("提问应当成功");

        assert_eq!(detail.turns.len(), 2);
        assert_eq!(detail.turns[0].turn_id, TurnId::new("t-1"));
        assert_eq!(detail.turns[1].text, "界世好你");
        assert_eq!(detail.summary.turn_count, 2);

        let reopened = Store::open(&storage).await.expect("应当能重新打开存储");
        let read_back = reopened
            .get_session(&workspace.workspace_id, &session.session_id)
            .await
            .expect("应当能读回会话");
        assert_eq!(read_back.turns.len(), 2);
        assert_eq!(read_back.turns[0].text, "你好世界");
        assert_eq!(read_back.turns[1].text, "界世好你");
    }

    /// 测试 `Ask` 按 `turn_id` 幂等：同一轮重复提问不会产生第二条回答。
    ///
    /// - 手段：建会话后对同一个 `turn_id` 连问两次，再换一个 `turn_id` 问一次。
    /// - 判断：前两次之后仍然只有两条消息（第二问没有追加任何东西）；
    ///   换了标识之后变成四条——去重只对"这一轮"生效，不会吞掉新的一轮。
    #[compio::test]
    async fn ask_is_idempotent_for_the_same_turn_() {
        let (_guard, storage, _runtime) = temp_dirs_();
        let store = Store::open(&storage).await.expect("应当能打开存储");
        let workspace = store
            .add_workspace("笔记", "/tmp/notes")
            .await
            .expect("应当能新增工作区");
        let session = store
            .create_session(&workspace.workspace_id, Some("第一问".to_string()), Vec::new())
            .await
            .expect("应当能新建会话");
        let service = KbService::new(store);

        let first = ask_(
            &service,
            &workspace.workspace_id,
            &session.session_id,
            "t-1",
            "你好",
        )
        .await;
        assert_eq!(first.turns.len(), 2);

        let again = ask_(
            &service,
            &workspace.workspace_id,
            &session.session_id,
            "t-1",
            "你好",
        )
        .await;
        assert_eq!(again.turns.len(), 2, "同一轮重复提问不应追加消息");

        let next = ask_(
            &service,
            &workspace.workspace_id,
            &session.session_id,
            "t-2",
            "再来一句",
        )
        .await;
        assert_eq!(next.turns.len(), 4, "换一轮应当照常追加");
    }

    /// 测试草稿落到服务端的流程：`CreateSession` 带第一个问题 → `Ask` 只补回答。
    ///
    /// - 手段：用 `CreateSession { title: None, turns: [用户提问] }` 建会话
    ///   （正是客户端草稿首次提问时发的形状），再用同一个 `turn_id` 调 `Ask`。
    /// - 判断：标题由 `kb_core` 从第一个问题推导（不是「新会话」）；`Ask` 之后
    ///   一共两条消息——用户回合没有被重复添加，问题与逆序回答都在。
    #[compio::test]
    async fn draft_flow_names_the_session_from_the_first_question_() {
        let (_guard, storage, _runtime) = temp_dirs_();
        let store = Store::open(&storage).await.expect("应当能打开存储");
        let workspace = store
            .add_workspace("笔记", "/tmp/notes")
            .await
            .expect("应当能新增工作区");

        let question = "你好世界";
        let summary = store
            .create_session(
                &workspace.workspace_id,
                None,
                vec![Turn {
                    turn_id: TurnId::new("t-1"),
                    role: Role::User,
                    text: question.to_string(),
                    reasoning: String::new(),
                    state: TurnState::Done,
                    tool_calls: Vec::new(),
                    usage: None,
                    notice: None,
                }],
            )
            .await
            .expect("带首问的会话应当能建");
        assert_eq!(summary.title, question, "名字应当取自第一个问题");

        let service = KbService::new(store);
        let detail = ask_(
            &service,
            &workspace.workspace_id,
            &summary.session_id,
            "t-1",
            question,
        )
        .await;

        assert_eq!(detail.turns.len(), 2, "问题不应当被重复添加");
        assert_eq!(detail.turns[0].text, question);
        assert_eq!(detail.turns[1].text, "界世好你");
    }

    /// 测试服务端的两个重命名都会落盘，并且拒绝空名字。
    ///
    /// - 手段：建工作区与会话后分别调 `rename_workspace` / `rename_session`，
    ///   再用同一个存储根目录重新 `Store::open` 读回；最后试一次空的工作区名。
    /// - 判断：返回与读回的名字 / 标题都是新的；空白工作区名是
    ///   `RpcError::Business(BadRequest)`——调用方输入不合法，不是服务端故障。
    #[compio::test]
    async fn rename_workspace_and_session_through_the_service_() {
        let (_guard, storage, _runtime) = temp_dirs_();
        let store = Store::open(&storage).await.expect("应当能打开存储");
        let workspace = store
            .add_workspace("旧名", "/tmp/notes")
            .await
            .expect("应当能新增工作区");
        let session = store
            .create_session(&workspace.workspace_id, Some("旧标题".to_string()), Vec::new())
            .await
            .expect("应当能新建会话");
        let service = KbService::new(store);

        let renamed_workspace = service
            .rename_workspace(workspace.workspace_id.clone(), "新名".to_string())
            .await
            .expect("工作区改名应当成功");
        assert_eq!(renamed_workspace.name, "新名");
        assert_eq!(renamed_workspace.workspace_id, workspace.workspace_id);

        let renamed_session = service
            .rename_session(
                workspace.workspace_id.clone(),
                session.session_id.clone(),
                "新标题".to_string(),
            )
            .await
            .expect("会话改名应当成功");
        assert_eq!(renamed_session.title, "新标题");

        let reopened = Store::open(&storage).await.expect("应当能重新打开存储");
        assert_eq!(
            reopened
                .get_workspace(&workspace.workspace_id)
                .await
                .expect("应当能读回工作区")
                .name,
            "新名"
        );
        assert_eq!(
            reopened
                .get_session(&workspace.workspace_id, &session.session_id)
                .await
                .expect("应当能读回会话")
                .summary
                .title,
            "新标题"
        );

        let error = service
            .rename_workspace(workspace.workspace_id.clone(), "   ".to_string())
            .await
            .expect_err("空白名字应当被拒绝");
        match error {
            RpcError::Business(reply) => assert_eq!(reply.code, ErrorCode::BadRequest),
            other => panic!("应当是业务错误，实际: {other:?}"),
        }
    }
}
