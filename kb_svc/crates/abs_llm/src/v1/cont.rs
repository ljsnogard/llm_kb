use core::ops::{Deref, Try};

use abs_str::string_view::TrStringView;
use serde::{Deserialize, Serialize};

pub trait TrMediaSource {
    type MimeStr: Deref<Target = str>;
    type Reader<'f>
    where
        Self: 'f;

    fn try_get_mime(&self) -> Option<Self::MimeStr>;

    fn try_read_bin(&mut self) -> impl Try<Output = Self::Reader<'_>>;
}

/// 输入内容。
///
/// Text 是最基础的能力；其它多模态能力属于通用 API 的自然扩展。
pub enum ContentPart<S, M>
where
    S: TrStringView<str>,
    M: TrMediaSource<MimeStr = S>,
{
    PlainText(S),
    ResourceUrl(S),
    MimeContent(M),
}

// ============================================================================
// 对话输入
// ============================================================================

/// 对话中的角色。
///
/// 这里只保留跨大多数 LLM API 都成立的语义。
/// 不在这里加入某个 Provider 独有的角色。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// 行为与约束，提示词
    System,

    /// 提问者，用户
    User,

    /// 模型，服务
    Assistant,

    /// 工具结果与中间状态
    Tool,
}

// ============================================================================
// 能力描述
// ============================================================================

/// 当前 LLM 实例能够提供的、面向应用的通用能力。
///
/// 这里的目标不是描述 Provider 的全部功能，而是帮助上层在运行时判断
/// 某项通用能力是否存在。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Capabilities {
    /// 是否能够连续返回增量输出。
    pub streaming: bool,

    /// 是否能够返回独立的 reasoning 内容。
    ///
    /// 注意：这不意味着 Provider 会返回“模型内部完整思维链”。
    /// 很多模型根本不会公开隐藏推理过程，或者只提供经过处理的
    /// reasoning summary。
    pub reasoning: bool,

    /// 是否支持多模态输入。
    pub multimodal_input: bool,

    /// 是否支持工具调用。
    pub tool_calling: bool,
}

// ============================================================================
// 输出
// ============================================================================

/// LLM 输出中的逻辑部分。
///
/// Reasoning 与 Answer 必须分开，而不能简单地把所有文字都放到一个
/// String 中。这样 UI 才可以选择样式，同时上下文管理的时候才有区分。
/// 同时，上层完全不需要知道这是 OpenAI、Anthropic 还是本地模型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicOutput {
    /// 面向最终答案的文本。
    Answer,

    /// Provider 愿意公开的 reasoning 内容，它不等同于“模型完整隐藏思维链”。
    Reasoning,

    /// 请求中要求执行的函数结果。
    FunctionCall,

    /// 对动态内容的搜索结果，例如互联网搜索，结果在长时间尺度内不稳定。
    /// 动态内容可能具有存储价值，如果需要审核回答内容的话。
    DynamicSearchCall,

    /// 对静态内容的搜索结果，例如文本内搜索，结果在较长时间尺度内是稳定的。
    /// 静态内容通常不需要额外存储，因为总是可以找到固定来源。
    StaticSearchCall,
}

/// 一段增量输出。
///
/// `text` 是一个“可立即显示的文本片段”，而不是一个严格意义上的
/// tokenizer token。
///
/// Provider 可能一次返回一个 token，也可能一次返回多个 token，
/// 甚至可能因为网络 buffering 返回更大的文本块。
pub trait TrTextDelta {
    type StrRepr: TrStringView<str>;

    fn logic(&self) -> LogicOutput;

    fn text(&self) -> Self::StrRepr;
}

/// 工具调用。
///
/// 工具调用属于跨多个现代 LLM 都存在的通用能力，因此可以进入公共
/// 抽象。但具体工具协议仍然由上层定义。
pub trait TrToolCall {
    type StrRepr: Deref<Target = str>;
    type Arguments: TrStringView<str>;

    fn id(&self) -> Self::StrRepr;

    fn name(&self) -> Self::StrRepr;

    fn arguments(&self) -> Self::Arguments;
}

// ============================================================================
// Streaming 事件
// ============================================================================

/// 流式响应中的事件。
///
/// Provider adapter 必须把自己的 streaming protocol 转换成这些事件。
/// 都应该转换成统一的 ResponseEvent。
#[derive(Debug, Clone)]
pub enum LlmRespEvent<D, C, U>
where
    D: TrTextDelta,
    C: TrToolCall,
    U: TrUsage,
{
    /// 回答部分新增文本。
    /// TextDelta 本身就能区分新增的文本是来自回答还是推理等
    TextDelta(D),

    /// 模型要求调用工具。
    ToolCall(C),

    /// 本次生成结束。
    Finished(FinishReason),

    /// 最终用量信息。
    ///
    /// 不保证所有 Provider 都能提供，因此不能依赖它来判断请求是否完成。
    Usage(U),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Completed,
    MaxTokens,
    Cancelled,
    ToolCall,
    Other,
}

/// Token 使用量。
///
/// 这是一个比较普遍的概念，但并不是所有 Provider 都一定提供精确值。
pub trait TrUsage {
    fn input_tokens(&self) -> Option<usize> {
        Option::None
    }

    fn output_tokens(&self) -> Option<usize> {
        Option::None
    }

    fn total_tokens(&self) -> Option<usize> {
        Option::None
    }
}
