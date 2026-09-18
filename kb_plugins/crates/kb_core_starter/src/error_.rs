//! 启动 `kb_core` 可能遇到的失败。
//!
//! 这里的错误全部落在**系统层**（起进程、读通知、等通知）：要么是进程起不来，
//! 要么是它没能给出可用的端点文件名。业务失败不在这里——那是
//! `abs_kb_svc` 协议里的事。

use std::path::PathBuf;

use thiserror::Error;

/// 启动 `kb_core` 失败。
#[derive(Debug, Error)]
pub enum LaunchError {
    /// 找不到可执行文件。
    #[error("kb_core 可执行文件不可用: {}（用 --kb-core <路径> 指定）", .0.display())]
    KbCoreNotUsable(PathBuf),

    /// `spawn` 失败。
    #[error("启动 kb_core 失败: {0}")]
    Spawn(#[source] std::io::Error),

    /// 拿不到 stdout 管道（理论上不会发生，因为是我们自己 piped 的）。
    #[error("拿不到 kb_core 的 stdout 管道")]
    NoStdout,

    /// 读通知失败。
    #[error("读取 kb_core 的 stdio 通知失败: {0}")]
    ReadNotice(#[source] std::io::Error),

    /// 子进程还没来得及通知就退出了。
    #[error("kb_core 在给出 stdio 通知之前就结束了（检查它的 stderr 日志）")]
    ExitedEarly,

    /// 通知不是合法 JSON。
    #[error("kb_core 的 stdio 通知不是合法 JSON: {0}")]
    BadNotice(#[source] serde_json::Error),

    /// 通知里那个字段缺失、或者取值为空。
    #[error("kb_core 的 stdio 通知里没有可用的 {0} 字段")]
    MissingField(&'static str),

    /// 专职读通知的线程起不来。
    #[error("读取 kb_core 通知的线程起不来: {0}")]
    NoticeThread(#[source] std::io::Error),

    /// 专职读通知的线程没有给出结果就结束了（通常是它自己 panic 了）。
    #[error("读取 kb_core 通知的线程没有给出结果就结束了")]
    NoticeThreadEnded,

    /// 等通知的过程中被调用方取消（例如"启动超时"）。
    ///
    /// 走到这条时子进程**已经被结束**，调用方不需要再收拾它。
    #[error("等 kb_core 的 stdio 通知时被取消")]
    Cancelled,
}

impl LaunchError {
    /// 是不是"等待被调用方取消"这一条。
    ///
    /// 把超时与真正的启动失败分开，界面才能说清"是它没起来，还是我们没等"。
    ///
    /// # Examples
    ///
    /// ```
    /// use kb_core_starter::LaunchError;
    ///
    /// assert!(LaunchError::Cancelled.is_cancelled());
    /// assert!(!LaunchError::ExitedEarly.is_cancelled());
    /// ```
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}
