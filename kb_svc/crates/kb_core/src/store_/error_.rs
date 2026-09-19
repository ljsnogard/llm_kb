//! 本地文件存储的错误类型。
//!
//! 按 `AGENTS.md` 第 9 条：实现 `core::error::Error`（`std::error::Error` 即它的
//! 再导出）、派生 `Debug`、给出清晰的 `Display`。
//!
//! 这里**只表达存储层自己的失败**，不掺入 IPC 或业务语义：下一轮接入请求/应答后，
//! 「工作区不存在」这类业务失败会由调用方翻译成
//! [`ErrorCode::NotFound`](abs_kb_svc::v1::desktop::ErrorCode::NotFound)，
//! 而「文件读不出来」这类真正的基础设施故障才应当向上冒泡。

use std::path::PathBuf;

use thiserror::Error;

/// 本地文件存储的失败。
#[derive(Debug, Error)]
pub enum StoreError {
    /// 文件系统操作失败，附带出错路径。
    #[error("文件操作失败: {}: {source}", path.display())]
    Io {
        /// 出错的文件或目录。
        path: PathBuf,

        /// 底层 I/O 错误。
        #[source]
        source: std::io::Error,
    },

    /// 某个 JSON 文件读不出来。
    ///
    /// 这通常意味着文件被手工改坏了，或者写入过程被强行中断。
    #[error("读取 {} 失败（文件可能被手工改坏）: {source}", path.display())]
    Decode {
        /// 出错的文件。
        path: PathBuf,

        /// 底层反序列化错误。
        #[source]
        source: serde_json::Error,
    },

    /// 待落盘的数据无法编码成 JSON。
    ///
    /// 对本模块使用的协议类型来说基本不会发生；保留它只是为了不把编码失败
    /// 伪装成 I/O 失败。
    #[error("编码为 JSON 失败: {0}")]
    Encode(#[source] serde_json::Error),

    /// 标识不能安全地当文件名用。
    #[error("{kind}标识不合法: {id:?}（只允许 ASCII 字母/数字/连字符/下划线，且不超过 128 字节）")]
    InvalidId {
        /// 对象类别（`工作区` / `会话`）。
        kind: &'static str,

        /// 被拒绝的标识。
        id: String,
    },

    /// 目标不存在。
    #[error("{kind}不存在: {id}")]
    NotFound {
        /// 对象类别（`工作区` / `会话`）。
        kind: &'static str,

        /// 目标标识。
        id: String,
    },

    /// 会话既没有消息，又只有默认名字——这样的会话不允许落盘。
    ///
    /// 用户口径："落盘的会话必须有真实对话内容，或者有默认名字以外的会话名。"
    /// 空会话因此必须先有个自定义名字，或者至少带上第一条消息。
    #[error("空会话需要一个自定义名字，或者至少带上第一条消息")]
    EmptySession,

    /// 名字为空（或只有空白）。
    #[error("{kind}名字不能为空")]
    EmptyName {
        /// 对象类别（目前只有 `工作区`）。
        kind: &'static str,
    },

    /// 交给阻塞线程池的任务没有正常返回（线程 panic 等）。
    #[error("后台阻塞任务异常结束: {0}")]
    BlockingTask(String),

    /// 调用在完成之前被取消令牌中止。
    ///
    /// 由 [`super::race_cancel_`] 产生：整个操作与取消信号赛跑，令牌先触发就
    /// 放弃尚未完成的等待。**已经落盘的部分不会被回滚**——文件操作是分步的，
    /// 最坏情况是留下一个不会被当作对象的 `*.json.tmp`。
    #[error("存储操作在完成前被取消")]
    Cancelled,
}
