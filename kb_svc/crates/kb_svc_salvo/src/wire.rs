//! 服务端 ↔ 浏览器、服务端 ↔ 插件 的线协议。
//!
//! 协议是 JSON 文本帧（WebSocket 的 text message），字段命名与 `abs_llm::v1`
//! 的词汇对齐：`delta` 对应 `LlmRespEvent::TextDelta` + `LogicOutput`，
//! `finished` 对应 `FinishReason`，`usage` 对应 `TrUsage`。
//!
//! # 为什么单独一个模块
//!
//! 两段协议（浏览器段、插件段）共用同一套「助手输出事件」。把它们放在一起可以
//! 保证「服务端只是转发，不做语义翻译」这一约束在类型层面成立。
//!
//! # 与 DSH 的对照
//!
//! DSH 的流式分片是 `{type:'text-delta'|'reasoning-delta', index, text}`。
//! 这里保留了「文本/推理分开」的语义，但把两者合并成一个 `delta` 帧加 `kind` 字段，
//! 便于与 [`crate::hub::LogicKind`] 一一对应。

use serde::{Deserialize, Serialize};

use crate::settings::LlmServiceConfig;

/// 助手输出文本的类别，对应 `abs_llm::v1::LogicOutput` 的应用侧子集。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogicKind {
    /// 面向最终答案的正文。
    Answer,

    /// Provider 公开的 reasoning 内容。
    Reasoning,
}

/// 生成结束的原因，对应 `abs_llm::v1::FinishReason`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// 正常生成结束。
    Completed,

    /// 达到输出上限。
    MaxTokens,

    /// 被用户取消。
    Cancelled,

    /// 模型要求调用工具。
    ToolCall,

    /// 其它原因。
    Other,
}

// ============================================================================
// 服务端 → 浏览器
// ============================================================================

/// 服务端推给浏览器的事件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// 连接建立后的第一条消息，用于让界面确定当前状态。
    Ready {
        /// 本次连接是否有插件在线。
        plugin_online: bool,
        /// 当前可用的服务标识列表。
        services: Vec<String>,
        /// 当前生效的服务标识（可能为 `None`）。
        active_service: Option<String>,
        /// 服务端版本，便于排查前后端不一致。
        server_version: String,
    },

    /// 某个 turn 已经开始生成。
    Started {
        /// turn 标识。
        turn_id: String,
        /// 实际使用的服务标识。
        service_id: String,
        /// 实际使用的模型名。
        model: String,
    },

    /// 一段增量文本。
    Delta {
        /// turn 标识。
        turn_id: String,
        /// 这段文本属于正文还是 reasoning。
        kind: LogicKind,
        /// 文本片段。
        text: String,
    },

    /// 模型要求调用工具。
    ToolCall {
        /// turn 标识。
        turn_id: String,
        /// 工具调用标识。
        id: String,
        /// 工具名。
        name: String,
        /// 参数（原始 JSON 文本）。
        arguments: String,
    },

    /// 用量信息。
    Usage {
        /// turn 标识。
        turn_id: String,
        /// 输入 token 数。
        input_tokens: Option<u64>,
        /// 输出 token 数。
        output_tokens: Option<u64>,
        /// 总 token 数。
        total_tokens: Option<u64>,
    },

    /// 本轮生成结束。
    Finished {
        /// turn 标识。
        turn_id: String,
        /// 结束原因。
        reason: FinishReason,
    },

    /// 请求或转发过程中的错误。
    Error {
        /// 相关 turn；与连接无关的错误为 `None`。
        turn_id: Option<String>,
        /// 错误类别，便于界面区分文案。
        code: ErrorCode,
        /// 面向用户的错误说明。
        message: String,
    },
}

/// 错误类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// 请求本身不合法（例如问题为空、turn 冲突）。
    BadRequest,

    /// 选定的服务不存在。
    UnknownService,

    /// 选定的服务还没有配置 API key。
    MissingApiKey,

    /// 插件当前不在线。
    PluginOffline,

    /// 服务端内部错误。
    Internal,
}

// ============================================================================
// 浏览器 → 服务端
// ============================================================================

/// 浏览器发给服务端的请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// 提出一个问题。
    Ask {
        /// 客户端生成的 turn 标识；缺省时由服务端生成。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        /// 问题正文。
        question: String,
        /// 使用的服务标识；缺省时用当前生效的服务。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        service_id: Option<String>,
    },

    /// 取消某一轮生成。
    Cancel {
        /// 要取消的 turn 标识。
        turn_id: String,
    },

    /// 切换当前生效的服务。
    UseService {
        /// 服务标识。
        service_id: String,
    },
}

// ============================================================================
// 服务端 ↔ 插件
// ============================================================================

/// 服务端下发给插件的指令。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginRequest {
    /// 请求插件就绪状态。
    Hello,

    /// 要求插件就某轮生成作答。
    Ask {
        /// turn 标识。
        turn_id: String,
        /// 服务标识（插件据此选择 provider 实现）。
        service_id: String,
        /// 该服务的完整配置，含 API key。
        service: LlmServiceConfig,
        /// 用户问题。
        question: String,
    },

    /// 取消某一轮生成。
    Cancel {
        /// turn 标识。
        turn_id: String,
    },

    /// 心跳。
    Ping,
}

/// 插件上报给服务端的事件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginEvent {
    /// 插件握手。
    Hello {
        /// 插件版本。
        plugin_version: String,
        /// 支持的 provider 列表。
        providers: Vec<String>,
    },

    /// 插件确认开始生成。
    Started {
        /// turn 标识。
        turn_id: String,
        /// 实际使用的模型名。
        model: String,
        /// 插件自报的能力（`abs_llm::v1::Capabilities` 的字符串化形式）。
        #[serde(default)]
        capabilities: Vec<String>,
    },

    /// 增量文本。
    Delta {
        /// turn 标识。
        turn_id: String,
        /// 文本类别。
        kind: LogicKind,
        /// 文本片段。
        text: String,
    },

    /// 工具调用。
    ToolCall {
        /// turn 标识。
        turn_id: String,
        /// 工具调用标识。
        id: String,
        /// 工具名。
        name: String,
        /// 参数（原始 JSON 文本）。
        arguments: String,
    },

    /// 用量。
    Usage {
        /// turn 标识。
        turn_id: String,
        /// 输入 token 数。
        input_tokens: Option<u64>,
        /// 输出 token 数。
        output_tokens: Option<u64>,
        /// 总 token 数。
        total_tokens: Option<u64>,
    },

    /// 生成结束。
    Finished {
        /// turn 标识。
        turn_id: String,
        /// 结束原因。
        reason: FinishReason,
    },

    /// 插件侧错误。
    Error {
        /// 相关 turn。
        turn_id: Option<String>,
        /// 错误说明。
        message: String,
    },

    /// 心跳应答。
    Pong,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试 `ClientMessage::Ask` 能正确往返序列化，且缺省字段被省略。
    ///
    /// - 手段：构造一个只有 `question` 的 `Ask`，序列化为 JSON 后再反序列化。
    /// - 判断：JSON 中的 `type` 为 `ask`；反序列化结果与原始值完全相等，
    ///   说明 `turn_id` / `service_id` 的 `#[serde(default)]` 生效。
    #[test]
    fn client_ask_round_trips_without_optional_fields() {
        let message = ClientMessage::Ask {
            turn_id: None,
            question: "你好".to_string(),
            service_id: None,
        };

        let json = serde_json::to_string(&message).expect("应当能序列化");
        assert!(json.contains(r#""type":"ask""#), "实际 JSON: {json}");
        assert!(!json.contains("turn_id"), "缺省字段不应出现: {json}");

        let parsed: ClientMessage = serde_json::from_str(&json).expect("应当能反序列化");
        assert_eq!(parsed, message);
    }

    /// 测试服务端增量帧的 JSON 形状与 `abs_llm` 词汇一致。
    ///
    /// - 手段：把一条 `Delta` 序列化为 JSON 字符串。
    /// - 判断：包含 `"type":"delta"`、`"kind":"reasoning"` 与 `"text"` 字段。
    #[test]
    fn server_delta_uses_kind_and_text() {
        let message = ServerMessage::Delta {
            turn_id: "t1".to_string(),
            kind: LogicKind::Reasoning,
            text: "思考中".to_string(),
        };

        let json = serde_json::to_string(&message).expect("应当能序列化");
        assert!(json.contains(r#""type":"delta""#), "实际 JSON: {json}");
        assert!(json.contains(r#""kind":"reasoning""#), "实际 JSON: {json}");
        assert!(json.contains(r#""text":"思考中""#), "实际 JSON: {json}");
    }

    /// 测试插件错误帧允许不带 turn 标识。
    ///
    /// - 手段：构造 `PluginEvent::Error { turn_id: None }` 并序列化。
    /// - 判断：JSON 中 `turn_id` 为 `null`，反序列化后仍为 `None`。
    #[test]
    fn plugin_error_allows_missing_turn() {
        let message = PluginEvent::Error {
            turn_id: None,
            message: "boom".to_string(),
        };

        let json = serde_json::to_string(&message).expect("应当能序列化");
        assert!(json.contains(r#""turn_id":null"#), "实际 JSON: {json}");

        let parsed: PluginEvent = serde_json::from_str(&json).expect("应当能反序列化");
        assert_eq!(parsed, message);
    }
}
