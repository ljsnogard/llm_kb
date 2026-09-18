//! 连接与查询的失败。
//!
//! 按 [`abs_kb_svc_v1_desktop::RpcError`] 的分法，**业务失败**（服务端明确说
//! "不行"）与**传输失败**（连不上、断了、被取消）分成不同变体：界面据此决定
//! "提示用户"还是"重试 / 换个连接方式"。

use abs_kb_svc_v1_desktop::{ErrorReply, RpcError};
use kb_client_config::ConfigError;
use kb_core_starter::LaunchError;
use kb_svc_servo_ipc::ServoIpcError;
use thiserror::Error;

use crate::tcp_::TcpError;

/// 连接、握手与业务调用可能出的错。
#[derive(Debug, Error)]
pub enum ClientError {
    /// 配置本身有问题（缺字段、版本不认识……）。
    #[error("配置有误: {0}")]
    Config(#[from] ConfigError),

    /// 启动 / 等 `kb_core` 公布端点失败。
    #[error("启动 kb_core 失败: {0}")]
    Launch(#[from] LaunchError),

    /// 本机 IPC 的传输失败。
    #[error("本机 IPC 传输失败: {0}")]
    Ipc(#[from] ServoIpcError),

    /// 远程 TCP 的传输失败。
    #[error("远程 TCP 传输失败: {0}")]
    Tcp(#[from] TcpError),

    /// 服务端明确回了"不行"。
    #[error("服务端拒绝: {0}")]
    Business(ErrorReply),

    /// 收到的应答与请求对不上（协议实现有问题）。
    #[error("期望 {expected} 应答，收到 {got}")]
    UnexpectedReply {
        /// 期望的应答种类。
        expected: &'static str,

        /// 实际收到的种类名。
        got: &'static str,
    },

    /// 调用被取消（多半是调用方的超时令牌到期）。
    #[error("调用被取消（例如超时）")]
    Cancelled,

    /// 搬阻塞调用的辅助线程起不来。
    #[error("起工作线程失败: {0}")]
    ThreadSpawn(#[source] std::io::Error),

    /// 搬阻塞调用的辅助线程没有给出结果就结束了（通常是它 panic 了）。
    #[error("连接线程异常结束")]
    WorkerLost,
}

impl ClientError {
    /// 是不是"调用方主动取消"这一条。
    ///
    /// 界面用不着把它当错误弹给用户——那是我们自己的超时策略。
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    /// 是不是传输层面的失败（值得重试 / 换一种连接方式）。
    pub fn is_transport(&self) -> bool {
        matches!(
            self,
            Self::Launch(_) | Self::Ipc(_) | Self::Tcp(_) | Self::Cancelled
        )
    }
}

/// 把本机 IPC 客户端的 `RpcError` 翻成 [`ClientError`]。
pub(crate) fn from_ipc_rpc_(error: RpcError<ServoIpcError>) -> ClientError {
    match error {
        RpcError::Business(reply) => ClientError::Business(reply),
        RpcError::Transport(ServoIpcError::Cancelled) => ClientError::Cancelled,
        RpcError::Transport(other) => ClientError::Ipc(other),
    }
}

/// 把远程 TCP 客户端的 `RpcError` 翻成 [`ClientError`]。
pub(crate) fn from_tcp_rpc_(error: RpcError<TcpError>) -> ClientError {
    match error {
        RpcError::Business(reply) => ClientError::Business(reply),
        RpcError::Transport(TcpError::Cancelled) => ClientError::Cancelled,
        RpcError::Transport(other) => ClientError::Tcp(other),
    }
}

/// 取应答变体的名字（只用于错误信息）。
pub(crate) fn reply_kind_(reply: &abs_kb_svc_v1_desktop::Reply) -> &'static str {
    use abs_kb_svc_v1_desktop::Reply;
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
