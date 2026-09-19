//! 一条已建立的连接（`kb_core` 视角）：三条通道的端点，以及请求派发。
//!
//! [`Connection::serve`] 是这个 crate 的"服务端主循环"：从请求流里取一条请求，
//! 派发给业务实现（[`TrKbService`]），把结果翻成协议里的 [`Reply`] 发回去。

use std::sync::{Mutex, MutexGuard};

use abs_kb_svc::v1::desktop::{
    ErrorCode, ErrorReply, Event, Reply, ReplyEnvelope, Request, RequestEnvelope, RpcError,
    TrKbService,
};
use futures_lite::StreamExt;
use ipc_channel::ipc::{IpcReceiver, IpcSender};

use super::error_::ServoIpcError;

/// 引导消息：客户端在连上之后交给 `kb_core` 的端点三元组。
///
/// 顺序是 `(请求接收端, 应答发送端, 事件发送端)`。
pub(crate) type Bootstrap = (
    IpcReceiver<RequestEnvelope>,
    IpcSender<ReplyEnvelope>,
    IpcSender<Event>,
);

/// 一条已建立的连接（服务端视角）。
///
/// 它是 `Send + Sync` 的：`serve` 只借用 `&self`，因此拿到连接之后仍然可以在
/// 别处用 [`Connection::send_event`] 主动推事件。
pub struct Connection {
    /// 请求接收端。用 `Mutex<Option<..>>` 是因为 `to_stream()` 会**消费**它，
    /// 而 `serve` 只想借用 `&self`。
    request_rx_: Mutex<Option<IpcReceiver<RequestEnvelope>>>,

    /// 应答发送端。
    reply_tx_: IpcSender<ReplyEnvelope>,

    /// 事件发送端。
    event_tx_: IpcSender<Event>,
}

impl Connection {
    /// 由 [`crate::Listener::accept`] 调用。
    pub(crate) fn new(
        request_rx: IpcReceiver<RequestEnvelope>,
        reply_tx: IpcSender<ReplyEnvelope>,
        event_tx: IpcSender<Event>,
    ) -> Self {
        Self {
            request_rx_: Mutex::new(Some(request_rx)),
            reply_tx_: reply_tx,
            event_tx_: event_tx,
        }
    }

    /// 给客户端推一条事件。
    ///
    /// 事件是"可能被丢弃也无所谓"的推送（见
    /// [`Event`](abs_kb_svc::v1::desktop::Event) 的定义），因此这里不做重试，
    /// 发不出去就如实报错。注意 ipc-channel 的通道是**无界**的：
    /// `send` 不会因为对端读得慢而阻塞，背压要由语义层自己管。
    ///
    /// # Errors
    ///
    /// 对端已断开时返回 [`ServoIpcError::Transport`]。
    pub fn send_event(&self, event: Event) -> Result<(), ServoIpcError> {
        self.event_tx_.send(event).map_err(ServoIpcError::from)
    }

    /// 逐个处理这条连接上的请求，直到对端断开。
    ///
    /// 这是**异步**的：请求流由 ipc-channel 的进程级 router 线程供给
    /// （`IpcReceiver::to_stream()`），调用方的执行器线程不会被阻塞。
    ///
    /// # Errors
    ///
    /// - [`ServoIpcError::AlreadyServing`]：同一条连接被 `serve` 了两次；
    /// - 请求解码失败、应答发不出去等传输层错误。
    ///
    /// 对端正常断开**不算错误**，函数返回 `Ok(())`。
    pub async fn serve<S>(&self, service: &S) -> Result<(), ServoIpcError>
    where
        S: TrKbService,
    {
        let request_rx = lock_(&self.request_rx_)
            .take()
            .ok_or(ServoIpcError::AlreadyServing)?;
        let mut requests = request_rx.to_stream();

        while let Some(item) = requests.next().await {
            let envelope = item?;
            let reply = dispatch_(service, envelope.request).await;
            self.reply_tx_
                .send(ReplyEnvelope::new(envelope.request_id, reply))?;
        }

        log::debug!("客户端已断开，本连接的 serve 结束");
        Ok(())
    }
}

/// 把一个请求派发给业务实现，并翻成协议里的应答。
///
/// 覆盖工作区、会话与生成（`Ask`）三个域；其余请求对应的 trait 还没落地，
/// 这里明确回一个 `BadRequest` 而不是静默丢弃——客户端必须能看出"这条请求
/// 现在还不支持"。
async fn dispatch_<S>(service: &S, request: Request) -> Reply
where
    S: TrKbService,
{
    match request {
        Request::Hello(client) => match service.hello(client).await {
            Ok(info) => Reply::Hello(info),
            Err(error) => business_reply_(error),
        },
        // `Ask` 的应答复用 `Reply::SessionDetail`：同步阶段它就是"提问之后的
        // 会话内容"，理由见 `TrGeneration` 的文档。
        Request::Ask(payload) => match service.ask(payload).await {
            Ok(detail) => Reply::SessionDetail(detail),
            Err(error) => business_reply_(error),
        },
        Request::ListWorkspaces => match service.list_workspaces().await {
            Ok(list) => Reply::WorkspaceList(list),
            Err(error) => business_reply_(error),
        },
        Request::AddWorkspace(payload) => {
            // `local_id` 是客户端的簿记字段，按协议原样回传。
            let local_id = payload.local_id.clone();
            match service.add_workspace(payload).await {
                Ok(workspace) => Reply::WorkspaceAdded {
                    local_id,
                    workspace,
                },
                Err(error) => business_reply_(error),
            }
        }
        Request::RemoveWorkspace { workspace_id } => {
            match service.remove_workspace(workspace_id).await {
                Ok(()) => Reply::Ack,
                Err(error) => business_reply_(error),
            }
        }
        Request::RenameWorkspace { workspace_id, name } => {
            match service.rename_workspace(workspace_id, name).await {
                Ok(workspace) => Reply::WorkspaceRenamed(workspace),
                Err(error) => business_reply_(error),
            }
        }
        Request::ListSessions { workspace_id } => match service.list_sessions(workspace_id).await {
            Ok(list) => Reply::SessionList(list),
            Err(error) => business_reply_(error),
        },
        Request::CreateSession(payload) => {
            let local_id = payload.local_id.clone();
            match service.create_session(payload).await {
                Ok(session) => Reply::SessionCreated { local_id, session },
                Err(error) => business_reply_(error),
            }
        }
        Request::GetSession {
            workspace_id,
            session_id,
        } => match service.get_session(workspace_id, session_id).await {
            Ok(detail) => Reply::SessionDetail(detail),
            Err(error) => business_reply_(error),
        },
        Request::RemoveSession {
            workspace_id,
            session_id,
        } => match service.remove_session(workspace_id, session_id).await {
            Ok(()) => Reply::Ack,
            Err(error) => business_reply_(error),
        },
        Request::RenameSession {
            workspace_id,
            session_id,
            title,
        } => match service.rename_session(workspace_id, session_id, title).await {
            Ok(session) => Reply::SessionRenamed(session),
            Err(error) => business_reply_(error),
        },
        other => Reply::Error(ErrorReply {
            code: ErrorCode::BadRequest,
            message: format!(
                "这条请求还没有实现（对应的按域 trait 尚未落地）: {}",
                request_kind_(&other)
            ),
        }),
    }
}

/// 把业务实现返回的失败翻成协议应答。
///
/// 两个分支的区别正是错误分层：
///
/// - [`RpcError::Business`] 本来就是协议里的业务错误，原样回传；
/// - [`RpcError::Transport`] 在**服务端**这一侧其实是"我这边出故障了"
///   （存储读不出来、内部状态坏了……），对客户端而言是 `Internal`。
fn business_reply_<E>(error: RpcError<E>) -> Reply
where
    E: core::error::Error,
{
    match error {
        RpcError::Business(reply) => Reply::Error(reply),
        RpcError::Transport(error) => Reply::Error(ErrorReply {
            code: ErrorCode::Internal,
            message: error.to_string(),
        }),
    }
}

/// 取请求变体的名字（只用于错误信息）。
fn request_kind_(request: &Request) -> &'static str {
    match request {
        Request::Hello(_) => "Hello",
        Request::Ask(_) => "Ask",
        Request::Cancel { .. } => "Cancel",
        Request::ListServices => "ListServices",
        Request::UpsertService(_) => "UpsertService",
        Request::RemoveService { .. } => "RemoveService",
        Request::UseService { .. } => "UseService",
        Request::ListWorkspaces => "ListWorkspaces",
        Request::AddWorkspace(_) => "AddWorkspace",
        Request::RemoveWorkspace { .. } => "RemoveWorkspace",
        Request::RenameWorkspace { .. } => "RenameWorkspace",
        Request::ListSessions { .. } => "ListSessions",
        Request::CreateSession(_) => "CreateSession",
        Request::RemoveSession { .. } => "RemoveSession",
        Request::RenameSession { .. } => "RenameSession",
        Request::GetSession { .. } => "GetSession",
        Request::ListDirectory { .. } => "ListDirectory",
    }
}

/// 取锁，忽略"中毒"：本 crate 的临界区里不会 panic，污染与否都不影响数据一致性。
fn lock_<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
