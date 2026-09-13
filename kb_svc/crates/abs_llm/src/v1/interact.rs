use core::{error, ops::Deref};

use abs_cancel::TrMayCancel;
use abs_str::string_view::TrStringView;
use anylr::TrEitherOf;

use crate::v1::cont::{
    Capabilities, ContentPart, FinishReason, LlmRespEvent, Role, TrMediaSource, TrTextDelta,
    TrToolCall, TrUsage,
};

/// 一条消息。
///
/// 使用 `ContentPart` 而不是简单的 String，是为了让接口未来可以
/// 自然支持图片、音频、文件等多模态输入，而不需要重新设计 Message。
pub trait TrMessage {
    type TextRepr: TrStringView<str>;
    type Resource: TrMediaSource<MimeStr = Self::TextRepr>;

    /// 构造一条纯文本的聊天内容，通常用于构造用户问题，或者从LLM答复中整理成对话上下文
    fn text(role: Role, text: impl Into<Self::TextRepr>) -> Self;

    /// 该消息是由谁发出的，或者由哪个角色发表的
    fn role(&self) -> Role;

    fn content(&self) -> impl IntoIterator<Item = ContentPart<Self::TextRepr, Self::Resource>>;
}

// ============================================================================
// 请求
// ============================================================================

/// 一次 LLM 对话请求。
///
/// 故意只包含跨 Provider 普遍存在的参数。
///
/// 不应该在这里加入诸如某个 Provider 特有的 sampling 参数、内部
/// reasoning effort 参数、缓存策略、特殊 header 等。否则所谓统一 API
/// 很快就会退化成“某个 Provider API 的 Rust wrapper”。
pub trait TrChatRequest {
    /// 用以存储表示对话中消息内容的字符串类型，通常 `String` 即可
    type StrRepr: Deref<Target = str>;

    /// 对话的目标模型的名字，通常用 `String`
    type ModelId: AsRef<str> + Clone + Eq;

    /// 一条对话消息
    type Message: TrMessage<TextRepr = Self::StrRepr>;

    /// 对话上下文
    type Conversation: TrConversation<StrRepr = Self::StrRepr, Message = Self::Message>;

    /// 对话的目标模型的元信息类型
    type Metadata;

    fn model_id(&self) -> Self::ModelId;

    fn conversation(&self) -> &Self::Conversation;

    fn requires_stream_output(&self) -> bool;

    fn max_output_tokens(&self) -> Option<usize>;

    fn temperature(&self) -> Option<f32>;

    fn metadata(&self) -> &Self::Metadata;
}

/// 一个可持续追加消息的对话。
///
/// 它只负责保存对话状态，不负责发送网络请求。
///
/// 这样 Provider 本身保持 stateless，而聊天状态由应用层管理。
pub trait TrConversation {
    type StrRepr: Deref<Target = str>;
    type Message: TrMessage<TextRepr = Self::StrRepr>;

    fn push(&mut self, message: Self::Message);

    fn messages(&self) -> impl IntoIterator<Item = Self::Message>;

    fn user(&mut self, text: impl Into<Self::StrRepr>) {
        self.push(Self::Message::text(Role::User, text))
    }

    fn assistant(&mut self, text: impl Into<Self::StrRepr>) {
        self.push(Self::Message::text(Role::Assistant, text))
    }
}

// ============================================================================
// 异步流
// ============================================================================

/// 一个 runtime-independent 的异步响应流。
///
/// 这里刻意没有使用 `Stream<Item = ...>`，也没有让调用者接触 `poll_next`。
///
/// Rust 标准库目前没有一个标准化的 async-stream trait，因此这里定义一个
/// 极小的 async pull interface：
/// Provider 内部当然可以使用 Tokio、async-std、smol、自己的 executor，
/// 但这些实现细节不会出现在这个 API 中。
pub trait TrResponseStream {
    type TextDelta: TrTextDelta;
    type ToolCall: TrToolCall;
    type Usage: TrUsage;

    type Output: TrEitherOf<
            Lt = Option<LlmRespEvent<Self::TextDelta, Self::ToolCall, Self::Usage>>,
            Rt = Self::Err,
        >;
    type Err: error::Error;

    /// 获取下一个响应事件。
    ///
    /// 返回 None 表示流已经正常结束。
    ///
    /// 这个设计还有一个重要性质：调用者可以在任意一次 `next().await`
    /// 后停止读取并直接丢弃 stream，从而自然终止后续处理。
    fn next<'a>(
        &'a mut self,
    ) -> impl TrMayCancel<'a, MayCancelOutput = Result<Self::Output, Self::Err>>;
}

// ============================================================================
// 非流式响应
// ============================================================================

/// 一次完整的 LLM 响应。
///
/// 它不是简单的 String，因为 reasoning、answer、tool call 等内容在
/// 语义上是不同的。
pub trait TrChatResponse {
    type StrRepr: Deref<Target = str>;
    type AnswerText: TrStringView<str>;
    type ReasoningText: TrStringView<str>;
    type Usage: TrUsage;

    fn answer(&self) -> Option<&Self::AnswerText>;

    fn reasoning(&self) -> Option<&Self::ReasoningText>;

    fn tool_calls(&self) -> impl IntoIterator<Item: TrToolCall>;

    fn finish_reason(&self) -> Option<FinishReason>;

    fn usage(&self) -> Option<&Self::Usage> {
        Option::None
    }
}

// ============================================================================
// LLM Service
// ============================================================================

/// 统一的 LLM Service 接口。
///
/// 这是整个抽象层最重要的 trait。
///
/// 使用关联类型 `Stream` 而不是 `Box<dyn ResponseStream>`，是因为
/// `async fn` trait 本身并不天然支持 dyn-compatible object。
/// 这样既保持了零运行时抽象，又允许每个 Provider 使用自己的 stream 类型。
pub trait TrLlmService {
    /// Provider 自己的 streaming implementation。
    type Stream: TrResponseStream;
    type ChatRequest: TrChatRequest;
    type ChatResponse: TrChatResponse;
    type Err: error::Error;

    type AskAsync<'f>: TrMayCancel<'f, MayCancelOutput: TrEitherOf<Lt = Self::ChatResponse, Rt = Self::Err>>
    where
        Self: 'f;

    type ChatAsync<'f>: TrMayCancel<'f, MayCancelOutput: TrEitherOf<Lt = Self::Stream, Rt = Self::Err>>
    where
        Self: 'f;

    /// 查询这个 LLM 实例提供哪些通用能力。
    ///
    /// 这是同步函数，因为它描述的是 client/model 本身的静态能力，
    /// 而不是一次网络请求。
    fn capabilities(&self) -> Capabilities;

    /// 获取一个完整响应。
    ///
    /// Provider 应该在内部消费 streaming protocol，并把最终结果组装成
    /// ChatResponse。
    ///
    /// 对不需要逐步显示回答的调用者，这是最简单的 API。
    fn ask_async<'f>(&'f self, request: Self::ChatRequest) -> Self::AskAsync<'f>;

    /// 发起流式请求。
    ///
    /// 这个函数本身只负责建立请求并取得 stream。
    /// 后续的模型输出通过 `ResponseStream::next()` 增量取得。
    fn chat_async<'f>(&'f self, request: Self::ChatRequest) -> Self::ChatAsync<'f>;
}
