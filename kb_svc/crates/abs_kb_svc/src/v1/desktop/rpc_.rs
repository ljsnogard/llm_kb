//! **按业务域拆分的异步 RPC 接口**。
//!
//! # 它是什么
//!
//! 把 `kb_core` 的能力按业务域（工作区、会话、设置、目录、生成……）拆成若干
//! 带异步方法的 trait。调用者（桌面客户端、插件、`kb_core` 自身）**面向这些
//! trait 编程**，具体传输由实现 crate 落地（当前选定的是 `kb_svc_servo_ipc`）。
//!
//! | 谁 | 做什么 |
//! | :--- | :--- |
//! | `kb_core` | 实现这些 trait，把业务逻辑（暂为本地文件存储）接上去 |
//! | `kb_svc_servo_ipc` | 客户端代理实现这些 trait（发请求、等应答）；同时把收到的请求派发给 `kb_core` 的实现 |
//! | 测试 | mock 实现（见 `tests/rpc_contract.rs`） |
//!
//! # 形状上的三条约定
//!
//! ## 1. 手写 GAT，而不是 `async fn`
//!
//! 每个方法都声明一个**受 [`TrMayCancel`] 约束的关联类型**，方法本身是同步的
//! （只负责把 future 造出来，不负责等待）：
//!
//! ```text
//! type ListWorkspaces<'f>: TrMayCancel<'f, MayCancelOutput = Result<应答, RpcError<错误>>>
//! where Self: 'f;
//!
//! fn list_workspaces<'f>(&'f self) -> Self::ListWorkspaces<'f>;
//! ```
//!
//! 这样做的原因有两个：
//!
//! - **`dyn` 兼容性与零装箱**：trait 里写 `async fn` 会让方法无法 `dyn`，
//!   而仓库既有的异步约定（`abs_cancel` v0.2 + `gen_mcf2`）就是 GAT 路线；
//! - **`gen_mcf2` 不能写进 trait**：该宏只能作用于**模块级自由函数**，
//!   所以能做的只能是"trait 声明 GAT、实现方用宏生成具体 future 类型来填"。
//!   该形状已由 `external/ipc-channel-poc/src/trait_spike.rs` 实测跑通。
//!
//! 实现方因此要在模块级写 `#[gen_mcf2::gen_may_cancel_future(Xxx, pub)] async fn …`，
//! 然后在 `impl` 里 `type Xxx<'f> = XxxAsync<'f, 'f>;`。
//! 这条路要求实现 crate 开启 nightly 的 `#![feature(impl_trait_in_assoc_type)]`。
//!
//! ## 2. 请求载荷进、应答载荷出
//!
//! trait 方法**收**协议里的请求载荷类型（[`AddWorkspaceRequest`] /
//! [`CreateSessionRequest`]…），**回**协议里的应答载荷类型（[`Workspace`] /
//! [`SessionSummary`]…），并省略纯客户端的簿记字段：
//!
//! | 协议里 | trait 里 |
//! | :--- | :--- |
//! | `Reply::WorkspaceAdded { local_id, workspace }` | 返回 [`Workspace`]。线上仍然带着 `local_id`（协议形状不变），只是 **trait 不暴露它**——调用方本来就知道自己发的是哪一个 |
//! | `Reply::SessionCreated { local_id, session }` | 返回 [`SessionSummary`]，同理 |
//! | `Reply::Ack` | 返回 `()` |
//! | `Reply::Error(ErrorReply)` | [`RpcError::Business`] |
//! | 编码失败 / 连接断开 / 对端消失 | [`RpcError::Transport`]（由实现方定义类型） |
//!
//! ## 3. 取消归到 [`RpcError::Transport`]
//!
//! [`TrMayCancel`] 要求"可取消"与"不可取消"两条路径的**输出类型相同**，
//! 因此取消也必须落在 [`RpcError`] 里表达。当前的取法是：**调用被中止、
//! 没有得到服务端的业务答复**，属于传输侧的失败，由实现方用自己的错误类型表示。
//! 这条约定在真实使用中若显得别扭（例如界面需要区分"用户主动取消"与"连接断了"），
//! 再考虑给 [`RpcError`] 增加第三个变体。
//!
//! # 尚未定义的部分
//!
//! - [`TrGeneration`]（`Ask` / `Cancel`）与事件订阅：它们返回的是**流**而不是单个
//!   future，形状要配合 `abs_async_iter::TrFlux` 定，见
//!   `dev-notes/kb_svc_servo_ipc-20260917-1548.md` §3.4；
//! - 设置（`TrSettingsService`）、目录（`TrDirectoryService`）、握手
//!   （`TrHandshake`）以及把它们组合起来的 `TrKbService`：签名已在上面那份
//!   dev-note 的 §3.4 列出，等 IPC 链路跑通后补。
//!
//! # 示例
//!
//! 本模块只定义 trait，没有可独立运行的示例；**可运行的**契约与 mock 实现见
//! `tests/rpc_contract.rs`（它用 `gen_mcf2` 展开 mock，并验证不可取消路径、
//! 可取消路径与业务错误映射）。

use abs_cancel::TrMayCancel;

use super::error_::ErrorReply;
use super::ids_::{SessionId, WorkspaceId};
use super::request_::{AddWorkspaceRequest, CreateSessionRequest};
use super::workspace_::{SessionDetail, SessionList, SessionSummary, Workspace, WorkspaceList};

/// 所有按域 RPC trait 的公共基底。
///
/// 它只回答一个问题：**实现方的错误类型是什么**。客户端代理是 ipc-channel 的
/// 传输错误，服务端逻辑是它自己的内部错误；把这件事收在一个关联类型上，
/// 是为了让"面向 trait 编程"的调用方不必知道实现是谁，也能拿到统一的错误形状。
pub trait TrKbEndpoint {
    /// 实现方的错误类型。
    ///
    /// 它承载的是**传输层**失败：连接断开、编解码失败、调用被取消……
    /// 业务失败不走这里，而是 [`RpcError::Business`]。
    type Error: core::error::Error + Send + Sync + 'static;
}

/// 一次 RPC 调用的失败。
///
/// 刻意把两类失败分成两个**变体**（`abs_kb_svc/README.md` §5 第 7 条：
/// 传输层错误与业务错误不得混在一个类型里），这样调用方一眼能看出
/// "该重试还是该提示用户"。
///
/// # 示例
///
/// ```
/// use abs_kb_svc::v1::desktop::{ErrorCode, ErrorReply, RpcError};
///
/// /// 假装的实现方错误。
/// #[derive(Debug)]
/// struct Dropped;
///
/// let business = RpcError::<Dropped>::Business(ErrorReply {
///     code: ErrorCode::NotFound,
///     message: "工作区不存在".to_string(),
/// });
/// assert_eq!(business.as_business().map(|error| error.code), Some(ErrorCode::NotFound));
///
/// let transport = RpcError::<Dropped>::Transport(Dropped);
/// assert!(transport.as_business().is_none());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcError<E> {
    /// 服务端明确答复"不行"：这是正常应答的一个分支，不该重试。
    Business(ErrorReply),

    /// 连接、编解码、对端消失、调用被取消……由实现方定义，通常值得重试。
    Transport(E),
}

impl<E> RpcError<E> {
    /// 取出业务错误；是传输错误时返回 `None`。
    pub fn as_business(&self) -> Option<&ErrorReply> {
        match self {
            Self::Business(error) => Some(error),
            Self::Transport(_) => None,
        }
    }

    /// 是不是传输层失败。
    pub fn is_transport(&self) -> bool {
        matches!(self, Self::Transport(_))
    }

    /// 把传输错误换一种类型，业务错误原样保留。
    ///
    /// 实现方在把内部错误往上抛时常用到它。
    pub fn map_transport<F>(self, map: impl FnOnce(E) -> F) -> RpcError<F> {
        match self {
            Self::Business(error) => RpcError::Business(error),
            Self::Transport(error) => RpcError::Transport(map(error)),
        }
    }
}

impl<E> core::fmt::Display for RpcError<E>
where
    E: core::fmt::Display,
{
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Business(error) => write!(formatter, "业务错误: {error}"),
            Self::Transport(error) => write!(formatter, "传输错误: {error}"),
        }
    }
}

impl<E> core::error::Error for RpcError<E>
where
    E: core::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Business(error) => Some(error),
            Self::Transport(error) => Some(error),
        }
    }
}

/// **工作区**域：工作区的增删查。
///
/// 对应协议里的 `Request::ListWorkspaces` / `AddWorkspace` / `RemoveWorkspace`。
pub trait TrWorkspaceService: TrKbEndpoint {
    /// [`TrWorkspaceService::list_workspaces`] 返回的可取消 future。
    type ListWorkspaces<'f>: TrMayCancel<'f, MayCancelOutput = Result<WorkspaceList, RpcError<Self::Error>>>
    where
        Self: 'f;

    /// [`TrWorkspaceService::add_workspace`] 返回的可取消 future。
    type AddWorkspace<'f>: TrMayCancel<'f, MayCancelOutput = Result<Workspace, RpcError<Self::Error>>>
    where
        Self: 'f;

    /// [`TrWorkspaceService::remove_workspace`] 返回的可取消 future。
    type RemoveWorkspace<'f>: TrMayCancel<'f, MayCancelOutput = Result<(), RpcError<Self::Error>>>
    where
        Self: 'f;

    /// 列出全部工作区。
    fn list_workspaces<'f>(&'f self) -> Self::ListWorkspaces<'f>;

    /// 登记一个（客户端本地先创建的）工作区，标识由 `kb_core` 分配。
    ///
    /// 参数里的 `local_id` 是调用方自己填的簿记字段：线上应答仍然按协议带着它
    /// 往返一次，但 **trait 的返回值里没有它**——调用方本来就知道自己发的是哪一个。
    fn add_workspace<'f>(&'f self, request: AddWorkspaceRequest) -> Self::AddWorkspace<'f>;

    /// 删除一个工作区（连同它名下的会话）。
    fn remove_workspace<'f>(&'f self, workspace_id: WorkspaceId) -> Self::RemoveWorkspace<'f>;
}

/// **会话**域：会话的增删查改。
///
/// 对应协议里的 `Request::ListSessions` / `CreateSession` / `RemoveSession` /
/// `GetSession`。注意列表只回**摘要**，正文要另外 [`TrSessionService::get_session`]，
/// 这与协议里"只发差异、不整棵树"的约定一致。
pub trait TrSessionService: TrKbEndpoint {
    /// [`TrSessionService::list_sessions`] 返回的可取消 future。
    type ListSessions<'f>: TrMayCancel<'f, MayCancelOutput = Result<SessionList, RpcError<Self::Error>>>
    where
        Self: 'f;

    /// [`TrSessionService::create_session`] 返回的可取消 future。
    type CreateSession<'f>: TrMayCancel<'f, MayCancelOutput = Result<SessionSummary, RpcError<Self::Error>>>
    where
        Self: 'f;

    /// [`TrSessionService::get_session`] 返回的可取消 future。
    type GetSession<'f>: TrMayCancel<'f, MayCancelOutput = Result<SessionDetail, RpcError<Self::Error>>>
    where
        Self: 'f;

    /// [`TrSessionService::remove_session`] 返回的可取消 future。
    type RemoveSession<'f>: TrMayCancel<'f, MayCancelOutput = Result<(), RpcError<Self::Error>>>
    where
        Self: 'f;

    /// 列出某个工作区下的会话（只含摘要）。
    fn list_sessions<'f>(&'f self, workspace_id: WorkspaceId) -> Self::ListSessions<'f>;

    /// 登记一个（客户端本地先创建的）会话，标识由 `kb_core` 分配。
    fn create_session<'f>(&'f self, request: CreateSessionRequest) -> Self::CreateSession<'f>;

    /// 读取一个会话的完整内容（摘要 + 全部消息）。
    fn get_session<'f>(
        &'f self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> Self::GetSession<'f>;

    /// 删除一个会话。
    fn remove_session<'f>(
        &'f self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> Self::RemoveSession<'f>;
}

/// 全部按域 trait 的**组合**：`kb_core` 的业务实现与 `kb_svc_servo_ipc` 的客户端
/// 代理都实现它，服务端派发层则只要求这一个约束。
///
/// 有一个 blanket 实现，因此实现方只要把各个按域 trait 实现了，就自动满足它。
/// 随着域的增加（设置 / 目录 / 生成 / 事件订阅），这里跟着加父 trait 即可。
pub trait TrKbService: TrKbEndpoint + TrWorkspaceService + TrSessionService {}

impl<T> TrKbService for T where T: TrKbEndpoint + TrWorkspaceService + TrSessionService {}
