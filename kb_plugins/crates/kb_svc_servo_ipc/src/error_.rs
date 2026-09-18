//! **传输层**失败。
//!
//! 按 `abs_kb_svc` README §5 第 7 条：传输层错误与业务错误必须分层。
//! 这里的类型会被填进
//! [`RpcError::Transport`](abs_kb_svc::v1::desktop::RpcError::Transport)；
//! 业务失败（目标不存在、请求不合法……）走
//! [`RpcError::Business`](abs_kb_svc::v1::desktop::RpcError::Business)，
//! 不混进本类型。

use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

/// 传输层的失败。
#[derive(Debug, Error)]
pub enum ServoIpcError {
    /// 端点名字文件的读写失败（发布、撤销、读取都算）。
    #[error("端点名字文件操作失败: {}: {source}", path.display())]
    NameFile {
        /// 名字文件路径。
        path: PathBuf,

        /// 底层 I/O 错误。
        #[source]
        source: std::io::Error,
    },

    /// `IpcOneShotServer::new()` 失败。
    #[error("创建 ipc-channel 端点失败: {0}")]
    CreateEndpoint(#[source] std::io::Error),

    /// `ipc::channel()` 失败（建不成一条通道）。
    #[error("创建 ipc-channel 通道失败: {0}")]
    CreateChannel(#[source] std::io::Error),

    /// 运行时目录准备失败。
    #[error("运行时目录准备失败: {}: {source}", path.display())]
    RuntimeDir {
        /// 出错的目录。
        path: PathBuf,

        /// 底层 I/O 错误。
        #[source]
        source: std::io::Error,
    },

    /// 客户端的路由线程起不来。
    #[error("无法启动客户端路由线程: {0}")]
    RouterSpawn(#[source] std::io::Error),

    /// ipc-channel 自己的传输错误（连接、发送、断开……）。
    #[error("ipc-channel 传输失败: {0}")]
    Transport(#[from] ipc_channel::IpcError),

    /// 消息编解码失败（postcard 解不开对端发来的东西）。
    #[error("消息编解码失败: {0}")]
    Decode(#[from] ipc_channel::SerDeError),

    /// 对端已关闭连接。
    ///
    /// 客户端的路由线程在读通道关闭时会唤醒所有还在等待的调用者，
    /// 它们拿到的就是这个错误。
    #[error("对端已关闭连接")]
    PeerClosed,

    /// 在给定时间内没能连上服务端。
    #[error("等待服务端端点超时（{} ms）", .timeout.as_millis())]
    ConnectTimeout {
        /// 实际等待了多久。
        timeout: Duration,
    },

    /// 一条连接已经在被 `serve` 处理了。
    #[error("这条连接已经在服务中（`Connection::serve` 只能调用一次）")]
    AlreadyServing,

    /// 服务端对一个请求回了不相干的应答。
    ///
    /// 正常实现不会出现；它意味着对端与本端的协议理解不一致。
    #[error("应答与请求不匹配: 期望 {expected}，实际收到 {got}")]
    UnexpectedReply {
        /// 期望的应答名字。
        expected: &'static str,

        /// 实际收到的应答名字。
        got: &'static str,
    },

    /// 调用在收到应答之前被取消令牌中止。
    ///
    /// 注意：请求**已经发出去了**，服务端仍会处理完；这条只是不再等它的应答。
    #[error("调用在收到应答前被取消")]
    Cancelled,
}
