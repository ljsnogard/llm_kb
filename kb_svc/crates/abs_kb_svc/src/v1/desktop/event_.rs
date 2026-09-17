//! `kb_core` 主动推给客户端的事件。
//!
//! 事件不与请求关联，且**可能被丢弃**（例如界面卡顿时）：
//! 需要可靠送达的内容应当放进 [`Reply`](crate::v1::desktop::Reply)。
//!
//! # 一轮生成的完整序列
//!
//! ```text
//! Event::TurnStarted   开始（含实际使用的服务与模型）
//! Event::Delta …       0..n 条增量（按 LogicOutput 分流到正文/推理）
//! Event::ToolCall …    0..n 次工具调用
//! Event::Usage         0..1 次用量
//! Event::TurnFinished  结束（含结束原因）
//! ```

use abs_llm::v1::cont::{Capabilities, FinishReason, LogicOutput};
use serde::{Deserialize, Serialize};

use super::content_::{TokenUsage, ToolCallRecord};
use super::error_::ErrorCode;
use super::handshake_::ServerState;
use super::ids_::{ServiceId, SessionId, TurnId};
use super::workspace_::SessionSummary;

/// 一轮生成已经开始。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnStarted {
    /// 回合标识。
    pub turn_id: TurnId,

    /// 该回合所属会话。
    pub session_id: SessionId,

    /// 实际使用的服务标识。
    pub service_id: ServiceId,

    /// 实际使用的模型名。
    pub model: String,

    /// 插件自报的能力。
    #[serde(default)]
    pub capabilities: Capabilities,
}

/// 一段增量文本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextDelta {
    /// 回合标识。
    pub turn_id: TurnId,

    /// 这段文本属于哪个逻辑部分（answer / reasoning / …）。
    pub logic: LogicOutput,

    /// 文本片段。
    pub text: String,
}

/// 模型要求调用工具。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallEvent {
    /// 回合标识。
    pub turn_id: TurnId,

    /// 调用记录。
    pub call: ToolCallRecord,
}

/// 用量信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageEvent {
    /// 回合标识。
    pub turn_id: TurnId,

    /// 用量明细。
    pub usage: TokenUsage,
}

/// 本轮生成结束。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnFinished {
    /// 回合标识。
    pub turn_id: TurnId,

    /// 结束原因；并非所有 provider 都会给出。
    #[serde(default)]
    pub reason: Option<FinishReason>,
}

/// 一条与生成过程有关的错误。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorEvent {
    /// 相关回合；与具体回合无关的错误为 `None`。
    #[serde(default)]
    pub turn_id: Option<TurnId>,

    /// 错误类别。
    pub code: ErrorCode,

    /// 面向用户的说明。
    pub message: String,
}

/// 一个会话的元信息发生了变化（标题、时间、条数）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionChanged {
    /// 变化后的摘要。
    pub summary: SessionSummary,
}

/// `kb_core` 主动推给桌面客户端的事件。
///
/// # 示例
///
/// ```
/// use abs_llm::v1::cont::LogicOutput;
/// use abs_kb_svc::v1::desktop::{Event, TextDelta, TurnId};
///
/// let event = Event::Delta(TextDelta {
///     turn_id: TurnId::new("t-1"),
///     logic: LogicOutput::Answer,
///     text: "你好".to_string(),
/// });
///
/// let json = serde_json::to_string(&event).expect("应当能序列化");
/// assert!(json.starts_with(r#"{"Delta""#), "实际 JSON: {json}");
/// assert!(json.contains(r#""logic":"answer""#), "实际 JSON: {json}");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Event {
    /// 握手完成后的第一条推送，以及每次状态刷新。
    Ready(ServerState),

    /// 一轮生成开始。
    TurnStarted(TurnStarted),

    /// 一段增量文本。
    Delta(TextDelta),

    /// 模型要求调用工具。
    ToolCall(ToolCallEvent),

    /// 用量信息。
    Usage(UsageEvent),

    /// 本轮生成结束。
    TurnFinished(TurnFinished),

    /// 生成过程相关的错误。
    Error(ErrorEvent),

    /// 服务端状态变化（插件上下线、服务配置变化）。
    StateChanged(ServerState),

    /// 会话元信息变化。
    SessionChanged(SessionChanged),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试一轮生成的完整事件序列能逐个往返，且复用 `abs_llm` 的词汇。
    ///
    /// - 手段：构造 `TurnStarted` → `Delta` → `Usage` → `TurnFinished` 四个事件，
    ///   逐个序列化再反序列化。
    /// - 判断：每个事件往返后与原值相等；`logic` 序列化为 `reasoning`、
    ///   `reason` 序列化为 `completed`，即与 `abs_llm::v1::cont` 的命名一致，
    ///   没有镜像枚举。
    #[test]
    fn streaming_events_reuse_abs_llm_vocabulary_() {
        let events = vec![
            Event::TurnStarted(TurnStarted {
                turn_id: TurnId::new("t-1"),
                session_id: SessionId::new("s-1"),
                service_id: ServiceId::new("deepseek"),
                model: "deepseek-chat".to_string(),
                capabilities: Capabilities::default(),
            }),
            Event::Delta(TextDelta {
                turn_id: TurnId::new("t-1"),
                logic: LogicOutput::Reasoning,
                text: "先想想".to_string(),
            }),
            Event::Usage(UsageEvent {
                turn_id: TurnId::new("t-1"),
                usage: TokenUsage {
                    input_tokens: None,
                    output_tokens: None,
                    total_tokens: Some(12),
                },
            }),
            Event::TurnFinished(TurnFinished {
                turn_id: TurnId::new("t-1"),
                reason: Some(FinishReason::Completed),
            }),
        ];

        let mut seen = Vec::new();
        for event in &events {
            let json = serde_json::to_string(event).expect("应当能序列化");
            seen.push(json.clone());
            let parsed: Event = serde_json::from_str(&json).expect("应当能反序列化");
            assert_eq!(&parsed, event);
        }

        assert!(
            seen[1].contains(r#""logic":"reasoning""#),
            "实际 JSON: {}",
            seen[1]
        );
        assert!(
            seen[2].contains(r#""total_tokens":12"#),
            "实际 JSON: {}",
            seen[2]
        );
        assert!(
            seen[3].contains(r#""reason":"completed""#),
            "实际 JSON: {}",
            seen[3]
        );
    }

    /// 测试与生成无关的错误其回合标识编码为 `null`。
    ///
    /// - 手段：构造 `turn_id: None` 的 `ErrorEvent` 并序列化。
    /// - 判断：JSON 中 `turn_id` 为 `null`（不省略，理由见
    ///   `optional_fields_are_always_encoded_`）；反序列化后仍为 `None`。
    #[test]
    fn connection_level_error_encodes_null_turn_id_() {
        let event = Event::Error(ErrorEvent {
            turn_id: None,
            code: ErrorCode::PluginOffline,
            message: "LLM 插件当前未连接".to_string(),
        });

        let json = serde_json::to_string(&event).expect("应当能序列化");
        assert!(json.contains(r#""turn_id":null"#), "实际 JSON: {json}");

        let parsed: Event = serde_json::from_str(&json).expect("应当能反序列化");
        assert_eq!(parsed, event);
    }
}
