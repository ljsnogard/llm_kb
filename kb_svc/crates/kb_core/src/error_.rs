//! 进程级失败。
//!
//! `kb_core` 是可执行程序，没有对外库接口；这个类型只是把"存储层失败"与
//! "IPC 层失败"收在一处，便于 `main` 统一决定退出码与日志。
//!
//! 业务错误**不在这里**：它是协议应答里的一个分支，由 [`crate::ipc_`] 翻成
//! `ErrorCode`（见 `abs_kb_svc` README §5 第 7 条的错误分层）。

use thiserror::Error;

use crate::store_::StoreError;

/// 进程级失败。
#[derive(Debug, Error)]
pub enum CoreError {
    /// 本地文件存储失败。
    #[error(transparent)]
    Store(#[from] StoreError),

    /// 进程间通信失败。
    #[error(transparent)]
    Ipc(#[from] kb_svc_servo_ipc::ServoIpcError),

    /// 交给阻塞线程池的任务没有正常返回。
    #[error("后台阻塞任务异常结束: {0}")]
    BlockingTask(String),

    /// 把系统层握手通知编码成 JSON 失败。
    ///
    /// 通知的类型是 `abs_kb_svc::v1::desktop::IpcReadyNotice`，字段全是字符串与
    /// 整数，正常不会失败；留着它是为了不在这里 `unwrap`。
    #[error("编码 kb_core 的 stdio 通知失败: {0}")]
    Encode(#[source] serde_json::Error),
}
