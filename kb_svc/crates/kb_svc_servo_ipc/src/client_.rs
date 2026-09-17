//! 客户端代理：用 ipc-channel 实现 [`abs_kb_svc`] 的按域 RPC trait。
//!
//! 每条 trait 方法做的事都一样：
//!
//! ```text
//! 造 RequestEnvelope（带一个新的 request_id）
//!   → 登记一个 oneshot 完成量
//!   → 把请求发给 kb_core
//!   → 等路由线程把对应 request_id 的应答送进完成量
//!   → 按 Reply 变体翻成业务载荷 / RpcError
//! ```
//!
//! # 阻塞都在哪
//!
//! 有一条**专用路由线程**：它阻塞在 `IpcReceiver::recv()` 上，把应答按
//! `request_id` 分发给还在等待的调用者。这样：
//!
//! - trait 方法返回的 future 里没有任何阻塞调用（只轮询 oneshot 与取消令牌）；
//! - 端点没有被交给调用方的执行器线程（`abs_kb_svc` README §5 第 1、2 条）。
//!
//! # 取消
//!
//! 等待应答时同时轮询取消令牌：令牌一触发就立刻收手，不会把调用方挂住。
//! **请求不会撤回**——协议里没有取消帧，服务端仍会处理完并把应答发回来，
//! 那条应答因为 `request_id` 已经无人认领而被丢弃。

use std::collections::HashMap;
use std::future::{Future, poll_fn};
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::Poll;
use std::time::Duration;

use abs_cancel::TrCancellationToken;
use abs_kb_svc::v1::desktop::{
    AddWorkspaceRequest, CreateSessionRequest, Event, Reply, ReplyEnvelope, Request,
    RequestEnvelope, RequestId, RpcError, SessionDetail, SessionId, SessionList, SessionSummary,
    TrKbEndpoint, TrSessionService, TrWorkspaceService, Workspace, WorkspaceId, WorkspaceList,
};
use futures_channel::oneshot;
use futures_lite::{Stream, StreamExt};
use gen_mcf2::gen_may_cancel_future;
use ipc_channel::ipc::{self, IpcReceiver, IpcSender};

use super::error_::ServoIpcError;
use super::rendezvous_::{DEFAULT_CONNECT_TIMEOUT, connect_with_retry_};

/// `kb_core` 的客户端代理。
///
/// 它**实现**了按域 RPC trait，所以调用方见到的就是那些 trait：
///
/// ```no_run
/// # use abs_kb_svc::v1::desktop::{TrWorkspaceService, TrKbEndpoint};
/// # use kb_svc_servo_ipc::Client;
/// # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
/// let client = Client::connect("/tmp/kb-demo")?;
/// let workspaces = client.list_workspaces().await?;
/// println!("{} 个工作区", workspaces.workspaces.len());
/// # Ok(())
/// # }
/// ```
///
/// 注意那条 `.await`：trait 方法本身是同步的，它返回一个实现了
/// [`IntoFuture`](std::future::IntoFuture) 的 future，`.await` 直接可用；
/// 需要取消时改成 `.may_cancel_with(token).await`（见
/// [`abs_cancel::TrMayCancel`]）。
pub struct Client {
    /// 上行：请求。
    request_tx_: IpcSender<RequestEnvelope>,

    /// 还没拿到应答的请求：`request_id` → 完成量。
    pending_: Arc<Mutex<HashMap<String, oneshot::Sender<ReplyEnvelope>>>>,

    /// 下行：事件。只能取一次（`to_stream` 会消费它）。
    event_rx_: Mutex<Option<IpcReceiver<Event>>>,

    /// 请求标识发号器。
    next_request_: AtomicU64,
}

impl Client {
    /// 连上 `runtime_dir` 下公布的 `kb_core` 端点，缺省最多等
    /// [`DEFAULT_CONNECT_TIMEOUT`]。
    ///
    /// # Errors
    ///
    /// 见 [`Client::connect_with_timeout`]。
    pub fn connect(runtime_dir: impl AsRef<Path>) -> Result<Self, ServoIpcError> {
        Self::connect_with_timeout(runtime_dir, DEFAULT_CONNECT_TIMEOUT)
    }

    /// 同上，但自定等待时长。
    ///
    /// 连接过程是**同步阻塞**的（读名字文件 + 重试 + 发引导消息），
    /// 因此应当从合适的线程调用，别放进异步任务的 poll 里。
    ///
    /// # Errors
    ///
    /// - [`ServoIpcError::ConnectTimeout`]：等待期内没能连上；
    /// - [`ServoIpcError::NameFile`]：名字文件读不了；
    /// - [`ServoIpcError::Transport`]：建通道、发引导消息失败；
    /// - [`ServoIpcError::RouterSpawn`]：路由线程起不来。
    pub fn connect_with_timeout(
        runtime_dir: impl AsRef<Path>,
        timeout: Duration,
    ) -> Result<Self, ServoIpcError> {
        let boot = connect_with_retry_(runtime_dir.as_ref(), timeout)?;

        let (request_tx, request_rx) =
            ipc::channel::<RequestEnvelope>().map_err(ServoIpcError::CreateChannel)?;
        let (reply_tx, reply_rx) =
            ipc::channel::<ReplyEnvelope>().map_err(ServoIpcError::CreateChannel)?;
        let (event_tx, event_rx) = ipc::channel::<Event>().map_err(ServoIpcError::CreateChannel)?;

        // 引导消息只发这一次；此后请求、应答、事件各走各的通道。
        boot.send((request_rx, reply_tx, event_tx))?;

        let pending = Arc::new(Mutex::new(HashMap::new()));
        let router_pending = Arc::clone(&pending);
        std::thread::Builder::new()
            .name("kb-svc-ipc-replies".to_string())
            .spawn(move || route_replies_(reply_rx, router_pending))
            .map_err(ServoIpcError::RouterSpawn)?;

        Ok(Self {
            request_tx_: request_tx,
            pending_: pending,
            event_rx_: Mutex::new(Some(event_rx)),
            next_request_: AtomicU64::new(0),
        })
    }

    /// 取出服务端主动推送的事件流（**只能取一次**，第二次返回 `None`）。
    ///
    /// 事件通道在连接建立时就已经开好；但事件是生成相关域（`Ask` / `Delta`…）
    /// 的产物，那些 trait 还没落地，所以这里先把流交出去、由调用方自己驱动。
    /// 取出来的流是运行时无关的 [`Stream`]。
    pub fn events(&self) -> Option<impl Stream<Item = Result<Event, ServoIpcError>>> {
        let receiver = lock_(&self.event_rx_).take()?;
        Some(
            receiver
                .to_stream()
                .map(|item| item.map_err(ServoIpcError::from)),
        )
    }

    /// 发一个请求并等它的应答。
    ///
    /// 三个分支对应三种结局：收到应答、路由线程退出（对端关闭）、取消令牌先触发。
    async fn request_<C>(
        &self,
        request: Request,
        cancel: C,
    ) -> Result<Reply, RpcError<ServoIpcError>>
    where
        C: TrCancellationToken,
    {
        // 已经取消就没必要打扰服务端。
        if cancel.is_cancelled() {
            return Err(RpcError::Transport(ServoIpcError::Cancelled));
        }

        let request_id = RequestId::new(format!(
            "q-{}",
            self.next_request_.fetch_add(1, Ordering::Relaxed)
        ));
        let (completion, waiting) = oneshot::channel();
        lock_(&self.pending_).insert(request_id.to_string(), completion);

        let envelope = RequestEnvelope::new(request_id.clone(), request);
        if let Err(error) = self.request_tx_.send(envelope) {
            lock_(&self.pending_).remove(request_id.as_str());
            return Err(RpcError::Transport(error.into()));
        }

        match await_reply_(waiting, cancel).await {
            Some(Ok(envelope)) => Ok(envelope.reply),
            // 完成量被丢弃 = 路由线程退出 = 对端没了。
            Some(Err(_dropped)) => Err(RpcError::Transport(ServoIpcError::PeerClosed)),
            None => {
                lock_(&self.pending_).remove(request_id.as_str());
                Err(RpcError::Transport(ServoIpcError::Cancelled))
            }
        }
    }
}

impl core::fmt::Debug for Client {
    /// 只报"还有多少请求在等应答"。
    ///
    /// 不打印通道端点（它们不可读），也不打印路由线程——所以用
    /// `finish_non_exhaustive` 明确告诉读者"这里省略了东西"。
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Client")
            .field("pending_requests", &lock_(&self.pending_).len())
            .finish_non_exhaustive()
    }
}

// ============================================================================
// 模块级 async fn：宏只能作用于它们，trait 的关联类型由它们生成的类型来填
// ============================================================================

/// [`TrWorkspaceService::list_workspaces`] 的代理实现。
#[gen_may_cancel_future(ListWorkspaces, pub)]
pub async fn list_workspaces_async<'c, C>(
    client: &'c Client,
    cancel: C,
) -> Result<WorkspaceList, RpcError<ServoIpcError>>
where
    C: TrCancellationToken,
{
    match client.request_(Request::ListWorkspaces, cancel).await? {
        Reply::WorkspaceList(list) => Ok(list),
        Reply::Error(error) => Err(RpcError::Business(error)),
        other => Err(RpcError::Transport(unexpected_("WorkspaceList", &other))),
    }
}

/// [`TrWorkspaceService::add_workspace`] 的代理实现。
#[gen_may_cancel_future(AddWorkspace, pub)]
pub async fn add_workspace_async<'c, C>(
    client: &'c Client,
    request: AddWorkspaceRequest,
    cancel: C,
) -> Result<Workspace, RpcError<ServoIpcError>>
where
    C: TrCancellationToken,
{
    match client
        .request_(Request::AddWorkspace(request), cancel)
        .await?
    {
        Reply::WorkspaceAdded { workspace, .. } => Ok(workspace),
        Reply::Error(error) => Err(RpcError::Business(error)),
        other => Err(RpcError::Transport(unexpected_("WorkspaceAdded", &other))),
    }
}

/// [`TrWorkspaceService::remove_workspace`] 的代理实现。
#[gen_may_cancel_future(RemoveWorkspace, pub)]
pub async fn remove_workspace_async<'c, C>(
    client: &'c Client,
    workspace_id: WorkspaceId,
    cancel: C,
) -> Result<(), RpcError<ServoIpcError>>
where
    C: TrCancellationToken,
{
    match client
        .request_(Request::RemoveWorkspace { workspace_id }, cancel)
        .await?
    {
        Reply::Ack => Ok(()),
        Reply::Error(error) => Err(RpcError::Business(error)),
        other => Err(RpcError::Transport(unexpected_("Ack", &other))),
    }
}

/// [`TrSessionService::list_sessions`] 的代理实现。
#[gen_may_cancel_future(ListSessions, pub)]
pub async fn list_sessions_async<'c, C>(
    client: &'c Client,
    workspace_id: WorkspaceId,
    cancel: C,
) -> Result<SessionList, RpcError<ServoIpcError>>
where
    C: TrCancellationToken,
{
    match client
        .request_(Request::ListSessions { workspace_id }, cancel)
        .await?
    {
        Reply::SessionList(list) => Ok(list),
        Reply::Error(error) => Err(RpcError::Business(error)),
        other => Err(RpcError::Transport(unexpected_("SessionList", &other))),
    }
}

/// [`TrSessionService::create_session`] 的代理实现。
#[gen_may_cancel_future(CreateSession, pub)]
pub async fn create_session_async<'c, C>(
    client: &'c Client,
    request: CreateSessionRequest,
    cancel: C,
) -> Result<SessionSummary, RpcError<ServoIpcError>>
where
    C: TrCancellationToken,
{
    match client
        .request_(Request::CreateSession(request), cancel)
        .await?
    {
        Reply::SessionCreated { session, .. } => Ok(session),
        Reply::Error(error) => Err(RpcError::Business(error)),
        other => Err(RpcError::Transport(unexpected_("SessionCreated", &other))),
    }
}

/// [`TrSessionService::get_session`] 的代理实现。
#[gen_may_cancel_future(GetSession, pub)]
pub async fn get_session_async<'c, C>(
    client: &'c Client,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    cancel: C,
) -> Result<SessionDetail, RpcError<ServoIpcError>>
where
    C: TrCancellationToken,
{
    match client
        .request_(
            Request::GetSession {
                workspace_id,
                session_id,
            },
            cancel,
        )
        .await?
    {
        Reply::SessionDetail(detail) => Ok(detail),
        Reply::Error(error) => Err(RpcError::Business(error)),
        other => Err(RpcError::Transport(unexpected_("SessionDetail", &other))),
    }
}

/// [`TrSessionService::remove_session`] 的代理实现。
#[gen_may_cancel_future(RemoveSession, pub)]
pub async fn remove_session_async<'c, C>(
    client: &'c Client,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    cancel: C,
) -> Result<(), RpcError<ServoIpcError>>
where
    C: TrCancellationToken,
{
    match client
        .request_(
            Request::RemoveSession {
                workspace_id,
                session_id,
            },
            cancel,
        )
        .await?
    {
        Reply::Ack => Ok(()),
        Reply::Error(error) => Err(RpcError::Business(error)),
        other => Err(RpcError::Transport(unexpected_("Ack", &other))),
    }
}

// ============================================================================
// 把生成的 future 填进 trait 的关联类型
// ============================================================================

impl TrKbEndpoint for Client {
    type Error = ServoIpcError;
}

impl TrWorkspaceService for Client {
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

impl TrSessionService for Client {
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
// 内部辅助
// ============================================================================

/// 路由线程：把应答按 `request_id` 交给对应的等待者。
///
/// 这是客户端侧唯一的阻塞收包点。通道关闭时清空 `pending_`，
/// 所有等待者会因为完成量被丢弃而拿到 [`ServoIpcError::PeerClosed`]。
fn route_replies_(
    reply_rx: IpcReceiver<ReplyEnvelope>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<ReplyEnvelope>>>>,
) {
    while let Ok(envelope) = reply_rx.recv() {
        let key = envelope.request_id.to_string();
        match lock_(&pending).remove(&key) {
            Some(completion) => {
                // 等待者可能已经取消并走开了；送不到也无所谓。
                let _ = completion.send(envelope);
            }
            None => log::debug!("收到无人认领的应答: {key}"),
        }
    }
    lock_(&pending).clear();
    log::debug!("应答通道已关闭，客户端路由线程退出");
}

/// 等应答，或在取消令牌触发时收手。
///
/// - `Some(Ok(..))`：收到应答；
/// - `Some(Err(..))`：完成量被丢弃（路由线程退出 / 对端关闭）；
/// - `None`：取消令牌先触发。
async fn await_reply_<C>(
    receiver: oneshot::Receiver<ReplyEnvelope>,
    cancel: C,
) -> Option<Result<ReplyEnvelope, oneshot::Canceled>>
where
    C: TrCancellationToken,
{
    let cancellation = cancel.cancellation();
    let mut receiver = receiver;
    let mut cancellation = std::pin::pin!(cancellation);

    poll_fn(move |context| {
        if let Poll::Ready(outcome) = Pin::new(&mut receiver).poll(context) {
            return Poll::Ready(Some(outcome));
        }
        if Pin::new(&mut cancellation).poll(context).is_ready() {
            return Poll::Ready(None);
        }
        Poll::Pending
    })
    .await
}

/// 把"收到不相干的应答"包成传输层错误。
fn unexpected_(expected: &'static str, got: &Reply) -> ServoIpcError {
    ServoIpcError::UnexpectedReply {
        expected,
        got: reply_kind_(got),
    }
}

/// 取应答变体的名字（只用于错误信息）。
fn reply_kind_(reply: &Reply) -> &'static str {
    match reply {
        Reply::Hello(_) => "Hello",
        Reply::Ack => "Ack",
        Reply::ServiceList(_) => "ServiceList",
        Reply::ServiceUpdated(_) => "ServiceUpdated",
        Reply::WorkspaceList(_) => "WorkspaceList",
        Reply::WorkspaceAdded { .. } => "WorkspaceAdded",
        Reply::SessionList(_) => "SessionList",
        Reply::SessionCreated { .. } => "SessionCreated",
        Reply::SessionDetail(_) => "SessionDetail",
        Reply::DirectoryListing(_) => "DirectoryListing",
        Reply::Error(_) => "Error",
    }
}

/// 取锁，忽略"中毒"：临界区里不会 panic。
fn lock_<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
