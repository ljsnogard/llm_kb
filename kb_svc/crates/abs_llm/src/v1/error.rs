use core::{
    error,
    fmt,
    ops::Deref,
    time::Duration,
};

use abs_mm::mem_alloc::CoreAlloc;
use mm_ptr::{
    Owned,
    x_deps::abs_mm,
};

// ============================================================================
// 基础错误
// ============================================================================

/// LLM 抽象层的统一错误。
///
/// Provider 可以在内部拥有非常复杂的错误类型，但到了这一层以后，
/// 上层只需要处理这些跨 Provider 都有意义的错误类别。
///
/// 如果应用确实需要诊断底层错误，可以通过 `ProviderError` 携带一个
/// provider 自己的、被擦除的错误对象，但不应该让正常业务逻辑依赖它。
pub enum LlmError<S, E = Owned<dyn error::Error + Send + Sync, CoreAlloc>>
where
    S: Deref<Target = str>,
    E: Deref<Target = dyn error::Error + Send + Sync>,
{
    /// 网络、连接、DNS、TLS 等传输层问题。
    Transport(E),

    /// Provider 返回了一个无法正常处理的响应。
    ///
    /// 这里使用字符串而不是 Provider 特定的错误类型，是为了避免
    /// 上层代码出现大量 `match OpenAiError { ... }` 之类的 Provider 耦合。
    InvalidResponse(S),

    /// 请求不符合当前模型或服务支持的能力。
    Unsupported(S),

    /// 服务端拒绝了请求，例如认证失败、权限不足、请求非法等。
    RequestRejected {
        message: S,
    },

    /// 服务端限流。
    RateLimited {
        /// 如果 Provider 能够提供可靠的建议等待时间，可以提供它。
        retry_after: Option<Duration>,
    },

    /// 服务端内部错误。
    Server {
        message: S,
    },

    /// 其它无法归类的错误。
    Other(E),
}

impl<S> fmt::Debug for LlmError<S>
where
    S: Deref<Target = str>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(e) => write!(f, "transport error: {}", e.deref()),
            Self::InvalidResponse(e) => write!(f, "invalid response: {}", e.deref()),
            Self::Unsupported(e) => write!(f, "unsupported operation: {}", e.deref()),
            Self::RequestRejected { message } => {
                write!(f, "request rejected: {}", message.deref())
            }
            Self::RateLimited { .. } => write!(f, "rate limited"),
            Self::Server { message } => write!(f, "server error: {}", message.deref()),
            Self::Other(e) => write!(f, "other error: {}", e.deref()),
        }
    }
}

impl<S> fmt::Display for LlmError<S>
where
    S: Deref<Target = str>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        <Self as fmt::Debug>::fmt(self, f)
    }
}

impl<S> error::Error for LlmError<S>
where
    S: Deref<Target = str>,
{}
