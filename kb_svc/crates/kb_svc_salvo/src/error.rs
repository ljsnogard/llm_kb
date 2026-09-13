//! `kb_svc_salvo` 的错误类型。

use core::fmt;

/// `kb_svc_salvo` 的统一错误类型。
///
/// PoC 阶段只需要表达「监听器创建/绑定失败」与「服务端运行失败」两类问题；
/// 正式实现时应按需扩展为更细的变体（线协议错误、子进程错误等）。
pub enum KbSvcError {
    /// 底层 I/O 失败，例如 socket 文件无法创建或监听失败。
    Io(std::io::Error),

    /// 服务端运行失败。
    Server(String),
}

impl fmt::Debug for KbSvcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Server(msg) => write!(f, "server error: {msg}"),
        }
    }
}

impl fmt::Display for KbSvcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Server(msg) => write!(f, "server error: {msg}"),
        }
    }
}

impl std::error::Error for KbSvcError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Server(_) => None,
        }
    }
}

impl From<std::io::Error> for KbSvcError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// `kb_svc_salvo` 的 `Result` 别名。
pub type KbSvcResult<T> = core::result::Result<T, KbSvcError>;
