//! 线协议：服务端 ↔ 浏览器、服务端 ↔ 插件。
//!
//! # 两条方向的格式并不相同（刻意如此）
//!
//! | 方向 | 格式 | 为什么 |
//! | :--- | :--- | :--- |
//! | 服务端 ↔ 插件 | 固定信封 + [`serde_json::Value`] 原始载荷 | agent 用 rig 说话，服务端**不解释**它，只做搬运（见 `dev-notes.md` §2.3） |
//! | 服务端 ↔ 浏览器 | 严格按 `abs_llm::v1` 的词汇 | 界面只应该看到统一抽象，见 `dev-notes.md` §2.4 |
//!
//! 两者的翻译由 `kb_rig_llm_v1_adapt` 完成（见 `dev-notes.md` §2.2），
//! 因此本模块**不引入 `abs_llm` 依赖**：它只负责「帧的外形」，语义映射属于 adapter。
//!
//! # `abs_llm::v1` 的词汇对照
//!
//! | 本模块 | `abs_llm::v1` |
//! | :--- | :--- |
//! | [`LogicOutput`] | `cont::LogicOutput`（五个变体逐一对应） |
//! | [`FinishReason`] | `cont::FinishReason` |
//! | [`Usage`] | `TrUsage`（三个可选计数） |
//! | [`Capabilities`] | `cont::Capabilities`（四个能力位） |
//! | [`ServerMessage::Delta`] 的 `logic` + `text` | `TrTextDelta::logic` + `text` |

use serde::{Deserialize, Serialize};

use crate::settings::LlmServiceConfig;

/// 助手输出文本所属的逻辑部分。
///
/// 与 `abs_llm::v1::cont::LogicOutput` 一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicOutput {
    /// 面向最终答案的文本。
    Answer,

    /// Provider 愿意公开的 reasoning 内容。
    Reasoning,

    /// 请求中要求执行的函数调用。
    FunctionCall,

    /// 动态内容搜索（结果不稳定）。
    DynamicSearchCall,

    /// 静态内容搜索（结果稳定）。
    StaticSearchCall,
}

/// 生成结束的原因；与 `abs_llm::v1::cont::FinishReason` 一一对应。
///
/// 用 `Option` 承载是因为**并非所有 provider 都会给出**结束原因。
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

/// Token 用量；对应 `abs_llm::v1::TrUsage`。
///
/// 三个字段都可缺省：provider 不保证提供精确值。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// 输入 token 数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,

    /// 输出 token 数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,

    /// 总 token 数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

/// 模型能力；对应 `abs_llm::v1::cont::Capabilities`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// 是否能够连续返回增量输出。
    #[serde(default)]
    pub streaming: bool,

    /// 是否能够返回独立的 reasoning 内容。
    #[serde(default)]
    pub reasoning: bool,

    /// 是否支持多模态输入。
    #[serde(default)]
    pub multimodal_input: bool,

    /// 是否支持工具调用。
    #[serde(default)]
    pub tool_calling: bool,
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
        /// 插件自报的能力。
        capabilities: Capabilities,
    },

    /// 一段增量文本；对应 `LlmRespEvent::TextDelta`。
    Delta {
        /// turn 标识。
        turn_id: String,
        /// 这段文本属于哪个逻辑部分（answer / reasoning / …）。
        logic: LogicOutput,
        /// 文本片段。
        text: String,
    },

    /// 模型要求调用工具；对应 `LlmRespEvent::ToolCall`。
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

    /// 用量信息；对应 `LlmRespEvent::Usage`。
    Usage {
        /// turn 标识。
        turn_id: String,
        /// 用量明细。
        usage: Usage,
    },

    /// 本轮生成结束；对应 `LlmRespEvent::Finished`。
    Finished {
        /// turn 标识。
        turn_id: String,
        /// 结束原因。
        reason: Option<FinishReason>,
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

    /// 插件上报了 provider 侧的错误。
    Provider,

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
///
/// `Ask` 里的 `service` 是完整配置（含 API key），由 agent 侧自行决定怎么用；
/// 服务端不理解 rig 的任何细节。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginRequest {
    /// 请求插件就绪状态。
    Hello,

    /// 要求插件就某轮生成作答。
    Ask {
        /// turn 标识。
        turn_id: String,
        /// 服务标识（agent 据此选择 provider 实现）。
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
///
/// # 「原样透传」的含义
///
/// [`PluginEvent::Raw`] 是唯一携带生成内容的帧：`payload` 就是 rig 产出的原始
/// JSON，服务端**不做解析**，交给 `kb_rig_llm_v1_adapt` 翻译。
///
/// 其余变体（握手、开始、结束、错误）是「信封级」信息，服务端需要它们来维护
/// turn 状态，因此在这里就有明确字段。
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

    /// 开始生成。
    Started {
        /// turn 标识。
        turn_id: String,
        /// 实际使用的模型名。
        model: String,
        /// 插件自报的能力。
        #[serde(default)]
        capabilities: Capabilities,
    },

    /// **原样的 rig 数据**：一条流式分片。
    ///
    /// `payload` 的结构由 rig 决定，也就由 `kb_rig_llm_v1_adapt` 解读。
    Raw {
        /// turn 标识。
        turn_id: String,
        /// rig 产出的原始 JSON。
        payload: serde_json::Value,
    },

    /// 生成结束。
    Finished {
        /// turn 标识。
        turn_id: String,
        /// 结束原因；插件可能无法给出。
        #[serde(default)]
        reason: Option<FinishReason>,
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

    /// 测试增量帧使用 `abs_llm` 的词汇（`logic` + `text`）。
    ///
    /// - 手段：把一条 `Delta` 序列化为 JSON 字符串。
    /// - 判断：包含 `"type":"delta"`、`"logic":"reasoning"` 与 `"text"`，
    ///   即与 `TrTextDelta::logic` / `text` 的命名一致。
    #[test]
    fn server_delta_uses_logic_and_text() {
        let message = ServerMessage::Delta {
            turn_id: "t1".to_string(),
            logic: LogicOutput::Reasoning,
            text: "思考中".to_string(),
        };

        let json = serde_json::to_string(&message).expect("应当能序列化");
        assert!(json.contains(r#""type":"delta""#), "实际 JSON: {json}");
        assert!(json.contains(r#""logic":"reasoning""#), "实际 JSON: {json}");
        assert!(json.contains(r#""text":"思考中""#), "实际 JSON: {json}");
    }

    /// 测试 `LogicOutput` 的五个变体与 `abs_llm::v1` 的命名一致。
    ///
    /// - 手段：逐个序列化五个变体。
    /// - 判断：得到 `answer` / `reasoning` / `function_call` /
    ///   `dynamic_search_call` / `static_search_call` 五个 snake_case 字符串。
    #[test]
    fn logic_output_covers_all_abs_llm_variants() {
        let cases = [
            (LogicOutput::Answer, "answer"),
            (LogicOutput::Reasoning, "reasoning"),
            (LogicOutput::FunctionCall, "function_call"),
            (LogicOutput::DynamicSearchCall, "dynamic_search_call"),
            (LogicOutput::StaticSearchCall, "static_search_call"),
        ];

        for (value, expected) in cases {
            let json = serde_json::to_string(&value).expect("应当能序列化");
            assert_eq!(json, format!("\"{expected}\""));
        }
    }

    /// 测试用量帧的三个计数都可缺省。
    ///
    /// - 手段：构造一个只填 `total_tokens` 的 `Usage`，序列化后反序列化。
    /// - 判断：JSON 中不含 `input_tokens` / `output_tokens`；解析回来的两个字段
    ///   为 `None`、`total_tokens` 保持原值。
    #[test]
    fn usage_omits_unknown_counters() {
        let message = ServerMessage::Usage {
            turn_id: "t1".to_string(),
            usage: Usage {
                input_tokens: None,
                output_tokens: None,
                total_tokens: Some(17),
            },
        };

        let json = serde_json::to_string(&message).expect("应当能序列化");
        assert!(!json.contains("input_tokens"), "实际 JSON: {json}");

        let parsed: ServerMessage = serde_json::from_str(&json).expect("应当能反序列化");
        match parsed {
            ServerMessage::Usage { usage, .. } => {
                assert_eq!(usage.input_tokens, None);
                assert_eq!(usage.total_tokens, Some(17));
            }
            other => panic!("应当是 usage，实际: {other:?}"),
        }
    }

    /// 测试插件原始帧可以承载任意 JSON 而不丢失字段。
    ///
    /// - 手段：构造一条 `PluginEvent::Raw`，`payload` 是一段嵌套的、含数组与
    ///   `null` 的对象；序列化后反序列化。
    /// - 判断：往返后 `payload` 与原始值完全相等，证明服务端不需要理解 rig 的结构。
    #[test]
    fn plugin_raw_payload_round_trips_unchanged() {
        let payload = serde_json::json!({
            "type": "text_delta",
            "index": 0,
            "text": "你好",
            "nested": { "a": [1, 2, null], "b": { "c": true } }
        });

        let message = PluginEvent::Raw {
            turn_id: "t1".to_string(),
            payload: payload.clone(),
        };

        let json = serde_json::to_string(&message).expect("应当能序列化");
        let parsed: PluginEvent = serde_json::from_str(&json).expect("应当能反序列化");

        match parsed {
            PluginEvent::Raw {
                payload: parsed, ..
            } => assert_eq!(parsed, payload),
            other => panic!("应当是 raw，实际: {other:?}"),
        }
    }

    /// 测试插件结束帧允许缺省的结束原因。
    ///
    /// - 手段：构造 `PluginEvent::Finished { reason: None }` 并序列化。
    /// - 判断：反序列化后 `reason` 仍为 `None`（`#[serde(default)]` 生效）。
    #[test]
    fn plugin_finished_allows_missing_reason() {
        let message = PluginEvent::Finished {
            turn_id: "t1".to_string(),
            reason: None,
        };

        let json = serde_json::to_string(&message).expect("应当能序列化");
        let parsed: PluginEvent = serde_json::from_str(&json).expect("应当能反序列化");
        assert_eq!(parsed, message);
    }
}
