//! 连接管理器：按一条 [`Connection`] 把客户端接上 `kb_core`，并暴露窄的查询接口。
//!
//! # 一次 `connect` 做两件事
//!
//! ```text
//! ① 系统层握手（找得到、连得上）
//!      local-launch → kb_core_starter::start（起进程 + 读就绪通知）+ 连 IPC
//!      local-attach → 连已在跑的 IPC
//!      tcp          → 连 kb_core_rproxy 的 TCP 端口
//! ② 应用层握手（谈得成）
//!      Request::Hello → Reply::Hello（双方身份 + 协议版本）
//! ```
//!
//! 两件事都做完才算"连上了"；任何一步失败都返回 [`ClientError`]，并且**不会**
//! 留下半个连接（`local-launch` 起来的子进程由 `Launched` 的 `Drop` 收掉）。
//!
//! # 阻塞都去哪了
//!
//! `Client::connect`（本机 IPC，含重试）与 `TcpClient::connect`（TCP）都是阻塞
//! 调用，本模块把它们搬到**专职线程**上，future 只轮询完成量 + 取消令牌
//! （`abs_kb_svc` README §5 第 1、2 条：future 里禁止阻塞）。于是这个 crate
//! 不挑异步运行时。

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use abs_cancel::{TrCancellationToken, TrMayCancel};
use abs_kb_svc_v1_desktop::{
    AddWorkspaceRequest, AskRequest, ClientInfo, CreateSessionRequest, PROTOCOL_VERSION, Reply,
    Request, RequestEnvelope, RequestId, ServerInfo, SessionDetail, SessionId, SessionList,
    SessionSummary, TrHandshake, Workspace, WorkspaceId, WorkspaceList,
};
use futures_channel::oneshot;
use gen_mcf2::gen_may_cancel_future;
use kb_client_config::Connection;
use kb_core_starter::{LaunchSpec, start};

use crate::error_::{ClientError, from_ipc_rpc_, from_tcp_rpc_, reply_kind_};
use crate::tcp_::TcpClient;

/// 应用层握手时自报的客户端名字。
pub const CLIENT_NAME: &str = "kb_admin_desktop";

/// 已经连上 `kb_core` 的客户端。
///
/// 它是 `Send + Sync` 的，可以放进全局单例（桌面端的 FRB 层就是这么用的）。
/// `local-launch` 起出来的子进程由它持有——这个值被丢弃时子进程也会被结束。
pub struct KbClient {
    /// 底层传输。
    transport_: Transport_,

    /// 应用层握手拿到的服务端身份。
    server_: ServerInfo,

    /// 这条连接的"单次请求"时限（来自配置，供调用方造超时令牌）。
    request_timeout_: Duration,

    /// 请求标识发号器（本连接内唯一）。
    next_request_: AtomicU64,
}

/// 三种传输形态。
enum Transport_ {
    /// 本机：自己起的 `kb_core` 子进程 + 连上它的 IPC 客户端。
    Launched {
        /// 子进程句柄；**持有的意义就是它的 `Drop`**（结束子进程）。
        launched_: kb_core_starter::Launched,

        /// IPC 客户端。
        client_: kb_svc_servo_ipc::Client,
    },

    /// 本机：附着到已经在跑的 `kb_core`。
    Attached(kb_svc_servo_ipc::Client),

    /// 远程：`kb_core_rproxy` 的 TCP。
    Tcp(TcpClient),
}

/// 按一条连接方式连上 `kb_core`（系统层 + 应用层握手）。
///
/// ```text
/// connect(&profile).await                          // 不可取消
/// connect(&profile).may_cancel_with(token).await   // 可取消 / 带超时
/// ```
///
/// **等多久由调用方决定**：本模块不自带定时器。界面通常这样用——
/// `TimeoutToken::after(profile.handshake_timeout())`（见 [`crate::TimeoutToken`]）。
///
/// # Errors
///
/// - [`ClientError::Cancelled`]：令牌在完成前触发（`local-launch` 起来的子进程
///   已经被收掉）；
/// - [`ClientError::Launch`]：起进程 / 读就绪通知失败；
/// - [`ClientError::Ipc`] / [`ClientError::Tcp`]：连不上或传输层断了；
/// - [`ClientError::Business`]：服务端在应用层握手里明确拒绝（例如协议版本不一致）。
pub fn connect(profile: &Connection) -> ConnectAsync<'_, '_> {
    ConnectAsync::new(profile)
}

/// [`connect`] 的实现体。
#[gen_may_cancel_future(Connect, pub)]
async fn connect_async<'s, C>(profile: &'s Connection, cancel: C) -> Result<KbClient, ClientError>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(ClientError::Cancelled);
    }

    // ── ① 系统层：找到 / 起一个 kb_core 并连上 ────────────────────────
    let transport = match profile {
        Connection::LocalLaunch {
            kb_core,
            runtime_dir,
            storage_dir,
            ..
        } => {
            let spec = LaunchSpec {
                kb_core: kb_core.clone(),
                runtime_dir: runtime_dir.clone(),
                storage_dir: storage_dir.clone(),
            };
            let launched = start(&spec)
                .may_cancel_with(cancel.child_token())
                .await
                .map_err(|error| match error {
                    kb_core_starter::LaunchError::Cancelled => ClientError::Cancelled,
                    other => ClientError::Launch(other),
                })?;

            let client = connect_ipc_(
                runtime_dir.clone(),
                profile.connect_timeout(),
                cancel.child_token(),
            )
            .await?;

            log::info!(
                "已启动并连上本机 kb_core（pid={:?}，运行时目录 {}）",
                launched.pid(),
                runtime_dir.display()
            );

            Transport_::Launched {
                launched_: launched,
                client_: client,
            }
        }

        Connection::LocalAttach { runtime_dir, .. } => {
            let client = connect_ipc_(
                runtime_dir.clone(),
                profile.connect_timeout(),
                cancel.child_token(),
            )
            .await?;
            log::info!(
                "已附着到本机 kb_core（运行时目录 {}）",
                runtime_dir.display()
            );
            Transport_::Attached(client)
        }

        Connection::Tcp { address, .. } => {
            let address = address.clone();
            let timeout = profile.connect_timeout();
            let client = await_blocking_(
                move || TcpClient::connect(&address, timeout).map_err(ClientError::Tcp),
                cancel.child_token(),
            )
            .await?;
            log::info!("已连上远程网关 {}", profile_address_(profile));
            Transport_::Tcp(client)
        }
    };

    // ── ② 应用层：Hello ───────────────────────────────────────────────
    let server = hello_(&transport, cancel.child_token()).await?;
    log::info!(
        "应用层握手成功：服务端 {}，协议 v{}",
        server.server_version,
        server.protocol_version
    );

    Ok(KbClient {
        transport_: transport,
        server_: server,
        request_timeout_: profile.request_timeout(),
        next_request_: AtomicU64::new(0),
    })
}

impl KbClient {
    /// 应用层握手拿到的服务端身份。
    pub fn server_info(&self) -> &ServerInfo {
        &self.server_
    }

    /// 这条连接建议的单次请求时限（来自配置）。
    ///
    /// 本 crate 不自己定时；调用方拿它造 [`crate::TimeoutToken`]。
    pub fn request_timeout(&self) -> Duration {
        self.request_timeout_
    }

    /// 是不是"本机"连接（IPC），而不是远程 TCP。
    pub fn is_local(&self) -> bool {
        !matches!(self.transport_, Transport_::Tcp(_))
    }

    /// 客户端自己起的那个 `kb_core` 的 pid；附着 / 远程连接返回 `None`。
    pub fn launched_pid(&self) -> Option<u32> {
        match &self.transport_ {
            Transport_::Launched { launched_, .. } => launched_.pid(),
            _ => None,
        }
    }

    /// 列出全部工作区。
    ///
    /// ```text
    /// client.list_workspaces().await
    /// client.list_workspaces().may_cancel_with(token).await
    /// ```
    pub fn list_workspaces<'f>(&'f self) -> ListWorkspacesAsync<'f, 'f> {
        ListWorkspacesAsync::new(self)
    }

    /// 列出某个工作区下的会话（只含摘要）。
    pub fn list_sessions<'f>(&'f self, workspace_id: WorkspaceId) -> ListSessionsAsync<'f, 'f> {
        ListSessionsAsync::new(self, workspace_id)
    }

    /// 新建（登记）一个工作区，标识由 `kb_core` 分配。
    ///
    /// `request.path` 是 **`kb_core` 所在主机上**的目录：客户端的本地文件系统
    /// 不参与这次操作，本 crate 只把名字与路径原样提交给服务端。
    pub fn add_workspace<'f>(&'f self, request: AddWorkspaceRequest) -> AddWorkspaceAsync<'f, 'f> {
        AddWorkspaceAsync::new(self, request)
    }

    /// 删除一个工作区；服务端会**级联删除**它名下的会话。
    pub fn remove_workspace<'f>(
        &'f self,
        workspace_id: WorkspaceId,
    ) -> RemoveWorkspaceAsync<'f, 'f> {
        RemoveWorkspaceAsync::new(self, workspace_id)
    }

    /// 在某个工作区下新建一个会话，标识由 `kb_core` 分配。
    ///
    /// [`CreateSessionRequest::turns`] 是客户端离线期间攒下的历史，连上之后新建
    /// 通常为空；留空时标题由服务端从标题或首条用户消息推导。
    pub fn create_session<'f>(
        &'f self,
        request: CreateSessionRequest,
    ) -> CreateSessionAsync<'f, 'f> {
        CreateSessionAsync::new(self, request)
    }

    /// 删除一个会话。
    pub fn remove_session<'f>(
        &'f self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> RemoveSessionAsync<'f, 'f> {
        RemoveSessionAsync::new(self, workspace_id, session_id)
    }

    /// 读取一个会话的完整内容（摘要 + 全部消息）。
    pub fn get_session<'f>(
        &'f self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> GetSessionAsync<'f, 'f> {
        GetSessionAsync::new(self, workspace_id, session_id)
    }

    /// 就某个会话提问，回来后拿到**提问之后**的会话内容。
    ///
    /// 现在是同步一问一答（详见 `abs_kb_svc_v1_desktop::TrGeneration` 的文档）：
    /// `kb_core` 里那个临时模拟的 LLM 会把问题逆序输出并落盘，因此这里的返回值
    /// 已经带着两条新消息。等流式生成落地后，这个入口会改成订阅事件。
    pub fn ask<'f>(&'f self, request: AskRequest) -> AskAsync<'f, 'f> {
        AskAsync::new(self, request)
    }

    /// 发一个请求并等它的应答（内部实现，两个传输共用）。
    async fn request_<C>(&self, request: Request, cancel: C) -> Result<Reply, ClientError>
    where
        C: TrCancellationToken,
    {
        let request_id = RequestId::new(format!(
            "q-{}",
            self.next_request_.fetch_add(1, Ordering::Relaxed)
        ));
        let envelope = RequestEnvelope::new(request_id, request);

        let reply = match &self.transport_ {
            Transport_::Launched { client_, .. } | Transport_::Attached(client_) => client_
                .send_envelope(envelope)
                .may_cancel_with(cancel)
                .await
                .map_err(from_ipc_rpc_)?,
            Transport_::Tcp(client_) => client_
                .send_envelope(envelope)
                .may_cancel_with(cancel)
                .await
                .map_err(from_tcp_rpc_)?,
        };

        Ok(reply.reply)
    }
}

impl core::fmt::Debug for KbClient {
    /// 只报身份与形态，不打印传输端点。
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("KbClient")
            .field("server_version", &self.server_.server_version)
            .field("protocol_version", &self.server_.protocol_version)
            .field("local", &self.is_local())
            .field("launched_pid", &self.launched_pid())
            .finish_non_exhaustive()
    }
}

/// [`KbClient::list_workspaces`] 的实现体。
#[gen_may_cancel_future(ListWorkspaces, pub)]
async fn list_workspaces_async<'c, C>(
    client: &'c KbClient,
    cancel: C,
) -> Result<WorkspaceList, ClientError>
where
    C: TrCancellationToken,
{
    match client.request_(Request::ListWorkspaces, cancel).await? {
        Reply::WorkspaceList(list) => Ok(list),
        Reply::Error(error) => Err(ClientError::Business(error)),
        other => Err(ClientError::UnexpectedReply {
            expected: "WorkspaceList",
            got: reply_kind_(&other),
        }),
    }
}

/// [`KbClient::list_sessions`] 的实现体。
#[gen_may_cancel_future(ListSessions, pub)]
async fn list_sessions_async<'c, C>(
    client: &'c KbClient,
    workspace_id: WorkspaceId,
    cancel: C,
) -> Result<SessionList, ClientError>
where
    C: TrCancellationToken,
{
    match client
        .request_(Request::ListSessions { workspace_id }, cancel)
        .await?
    {
        Reply::SessionList(list) => Ok(list),
        Reply::Error(error) => Err(ClientError::Business(error)),
        other => Err(ClientError::UnexpectedReply {
            expected: "SessionList",
            got: reply_kind_(&other),
        }),
    }
}

/// [`KbClient::add_workspace`] 的实现体。
#[gen_may_cancel_future(AddWorkspace, pub)]
async fn add_workspace_async<'c, C>(
    client: &'c KbClient,
    request: AddWorkspaceRequest,
    cancel: C,
) -> Result<Workspace, ClientError>
where
    C: TrCancellationToken,
{
    match client
        .request_(Request::AddWorkspace(request), cancel)
        .await?
    {
        // `local_id` 是调用方的簿记字段：线上应答仍然带着它往返一次，但这里
        // 只回服务端分配好的工作区（调用方本来就知道自己发的是哪一个）。
        Reply::WorkspaceAdded { workspace, .. } => Ok(workspace),
        Reply::Error(error) => Err(ClientError::Business(error)),
        other => Err(ClientError::UnexpectedReply {
            expected: "WorkspaceAdded",
            got: reply_kind_(&other),
        }),
    }
}

/// [`KbClient::remove_workspace`] 的实现体。
#[gen_may_cancel_future(RemoveWorkspace, pub)]
async fn remove_workspace_async<'c, C>(
    client: &'c KbClient,
    workspace_id: WorkspaceId,
    cancel: C,
) -> Result<(), ClientError>
where
    C: TrCancellationToken,
{
    match client
        .request_(Request::RemoveWorkspace { workspace_id }, cancel)
        .await?
    {
        Reply::Ack => Ok(()),
        Reply::Error(error) => Err(ClientError::Business(error)),
        other => Err(ClientError::UnexpectedReply {
            expected: "Ack",
            got: reply_kind_(&other),
        }),
    }
}

/// [`KbClient::create_session`] 的实现体。
#[gen_may_cancel_future(CreateSession, pub)]
async fn create_session_async<'c, C>(
    client: &'c KbClient,
    request: CreateSessionRequest,
    cancel: C,
) -> Result<SessionSummary, ClientError>
where
    C: TrCancellationToken,
{
    match client
        .request_(Request::CreateSession(request), cancel)
        .await?
    {
        // 与 `add_workspace` 同理：`local_id` 不上抛。
        Reply::SessionCreated { session, .. } => Ok(session),
        Reply::Error(error) => Err(ClientError::Business(error)),
        other => Err(ClientError::UnexpectedReply {
            expected: "SessionCreated",
            got: reply_kind_(&other),
        }),
    }
}

/// [`KbClient::remove_session`] 的实现体。
#[gen_may_cancel_future(RemoveSession, pub)]
async fn remove_session_async<'c, C>(
    client: &'c KbClient,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    cancel: C,
) -> Result<(), ClientError>
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
        Reply::Error(error) => Err(ClientError::Business(error)),
        other => Err(ClientError::UnexpectedReply {
            expected: "Ack",
            got: reply_kind_(&other),
        }),
    }
}

/// [`KbClient::get_session`] 的实现体。
#[gen_may_cancel_future(GetSession, pub)]
async fn get_session_async<'c, C>(
    client: &'c KbClient,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    cancel: C,
) -> Result<SessionDetail, ClientError>
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
        Reply::Error(error) => Err(ClientError::Business(error)),
        other => Err(ClientError::UnexpectedReply {
            expected: "SessionDetail",
            got: reply_kind_(&other),
        }),
    }
}

/// [`KbClient::ask`] 的实现体。
///
/// `Ask` 的应答复用 `Reply::SessionDetail`（同步一问一答阶段的形状，见
/// [`TrGeneration`](abs_kb_svc_v1_desktop::TrGeneration) 的文档）。
#[gen_may_cancel_future(Ask, pub)]
async fn ask_async<'c, C>(
    client: &'c KbClient,
    request: AskRequest,
    cancel: C,
) -> Result<SessionDetail, ClientError>
where
    C: TrCancellationToken,
{
    match client.request_(Request::Ask(request), cancel).await? {
        Reply::SessionDetail(detail) => Ok(detail),
        Reply::Error(error) => Err(ClientError::Business(error)),
        other => Err(ClientError::UnexpectedReply {
            expected: "SessionDetail",
            got: reply_kind_(&other),
        }),
    }
}

/// 给"首次运行"的界面一个**缺省的本机连接方式**。
///
/// - 运行时目录 / 存储目录用 `kb_core` 自己的缺省规则（[`kb_client_config::default_runtime_dir`]）；
/// - `kb_core` 可执行文件用 `kb_core_starter::default_kb_core_path()` 猜（"与本可执行
///   文件同目录"）。**界面不能假设它猜得对**：Flutter 打包出来的 App 里这个目录是
///   runner 所在处，`kb-core` 通常不在那儿。猜不到时这里是空路径，界面应当把它当成
///   "必填"让用户自己选。
pub fn suggested_local_launch(name: impl Into<String>) -> Connection {
    let runtime_dir = kb_client_config::default_runtime_dir();
    let storage_dir = kb_client_config::default_storage_dir(&runtime_dir);
    let kb_core = kb_core_starter::default_kb_core_path().unwrap_or_default();
    Connection::local_launch(name, kb_core, runtime_dir, storage_dir)
}

/// 应用层握手：自报身份，取回服务端身份。
async fn hello_<C>(transport: &Transport_, cancel: C) -> Result<ServerInfo, ClientError>
where
    C: TrCancellationToken,
{
    let info = ClientInfo {
        client_name: CLIENT_NAME.to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: PROTOCOL_VERSION,
    };

    match transport {
        Transport_::Launched { client_, .. } | Transport_::Attached(client_) => client_
            .hello(info)
            .may_cancel_with(cancel)
            .await
            .map_err(from_ipc_rpc_),

        Transport_::Tcp(client_) => match client_
            .send_request(Request::Hello(info))
            .may_cancel_with(cancel)
            .await
            .map_err(from_tcp_rpc_)?
            .reply
        {
            Reply::Hello(server) => Ok(server),
            Reply::Error(error) => Err(ClientError::Business(error)),
            other => Err(ClientError::UnexpectedReply {
                expected: "Hello",
                got: reply_kind_(&other),
            }),
        },
    }
}

/// 连本机 IPC（阻塞调用搬到专职线程上）。
async fn connect_ipc_<C>(
    runtime_dir: std::path::PathBuf,
    timeout: Duration,
    cancel: C,
) -> Result<kb_svc_servo_ipc::Client, ClientError>
where
    C: TrCancellationToken,
{
    await_blocking_(
        move || {
            kb_svc_servo_ipc::Client::connect_with_timeout(&runtime_dir, timeout)
                .map_err(ClientError::Ipc)
        },
        cancel,
    )
    .await
}

/// 把一次阻塞调用搬到专职线程上，并在等待期间支持取消。
///
/// **取消之后那条线程还会跑完**（我们没法打断阻塞的 `connect`），它产出的结果
/// 会因为完成量被丢弃而丢掉——对本 crate 来说这是正确的收场：`Client` / 子进程
/// 都在结果里，丢掉就等于收掉。
async fn await_blocking_<T, F, C>(work: F, cancel: C) -> Result<T, ClientError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ClientError> + Send + 'static,
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        return Err(ClientError::Cancelled);
    }

    let (sender, receiver) = oneshot::channel();
    std::thread::Builder::new()
        .name("kb-client-connect".to_string())
        .spawn(move || {
            let _ = sender.send(work());
        })
        .map_err(ClientError::ThreadSpawn)?;

    let cancellation = cancel.cancellation();
    let mut receiver = receiver;
    let mut cancellation = std::pin::pin!(cancellation);

    std::future::poll_fn(move |context| {
        if let std::task::Poll::Ready(outcome) = std::pin::Pin::new(&mut receiver).poll(context) {
            return std::task::Poll::Ready(match outcome {
                Ok(result) => result,
                // 完成量被丢弃 = 那条线程异常结束（通常是 panic）。
                Err(_dropped) => Err(ClientError::WorkerLost),
            });
        }
        if std::pin::Pin::new(&mut cancellation)
            .poll(context)
            .is_ready()
        {
            return std::task::Poll::Ready(Err(ClientError::Cancelled));
        }
        std::task::Poll::Pending
    })
    .await
}

/// 取一条连接方式的"地址"（日志用）：本机给运行时目录，远程给地址。
fn profile_address_(profile: &Connection) -> String {
    match profile {
        Connection::LocalLaunch { runtime_dir, .. }
        | Connection::LocalAttach { runtime_dir, .. } => runtime_dir.display().to_string(),
        Connection::Tcp { address, .. } => address.clone(),
    }
}
