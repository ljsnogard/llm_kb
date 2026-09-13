//! `kb_svc_salvo` 的错误类型。

use core::fmt;

/// `kb_svc_salvo` 的统一错误类型。
///
/// 目前只覆盖「I/O / 配置 / 服务端」三类；随着会话与插件监管落地，会按需扩展。
pub enum KbSvcError {
    /// 底层 I/O 失败，例如 socket 文件无法创建、配置文件无法写入。
    Io(std::io::Error),

    /// 配置内容不合法，例如 TOML 解析失败、字段类型不对。
    Config(String),

    /// 服务端运行失败。
    Server(String),
}

impl fmt::Debug for KbSvcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Config(msg) => write!(f, "config error: {msg}"),
            Self::Server(msg) => write!(f, "server error: {msg}"),
        }
    }
}

impl fmt::Display for KbSvcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Config(msg) => write!(f, "config error: {msg}"),
            Self::Server(msg) => write!(f, "server error: {msg}"),
        }
    }
}

impl core::error::Error for KbSvcError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Config(_) | Self::Server(_) => None,
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
