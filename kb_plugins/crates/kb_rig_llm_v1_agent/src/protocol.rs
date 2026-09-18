//! agent 侧的插件线协议。
//!
//! 服务端的对应定义在 `kb_svc_salvo::wire` 中。agent 不依赖服务端 crate，因此
//! 在这里镜像「信封级」协议；内容分片仍然走 [`PluginEvent::Raw`] 里的 rig 原始
//! JSON，由 `kb_svc_salvo` 进程内的 `kb_rig_llm_v1_adapt` 翻译。
//!
//! `abs_llm` 引入 serde 后，本模块的语义字段（[`Capabilities`] 与
//! [`FinishReason`]）直接复用 `abs_llm::v1::cont` 的类型，不再维护重复的
//! enum / struct；JSON 形状仍与服务端的 `wire.rs` 保持一致。

use abs_llm::v1::cont::{Capabilities, FinishReason};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 服务端下发给插件的指令。
///
/// 字段与 `kb_svc_salvo::wire::PluginRequest` 对齐；`service` 是完整服务配置，
/// 含 API key。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginRequest {
    /// 请求插件就绪状态。
    Hello,

    /// 要求插件就某轮生成作答。
    Ask {
        /// turn 标识。
        turn_id: String,
        /// 服务标识。
        service_id: String,
        /// 该服务的完整配置。
        service: LlmServiceConfig,
        /// 用户问题。
        question: String,
    },

    /// 取消某一轮生成。
    Cancel {
        /// 要取消的 turn 标识。
        turn_id: String,
    },

    /// 心跳。
    Ping,
}

/// 插件上报给服务端的事件。
///
/// 字段与 `kb_svc_salvo::wire::PluginEvent` 对齐。`Started.capabilities` 与
/// `Finished.reason` 直接使用 `abs_llm` 的 serde 类型，避免在 agent 侧再定义
/// 一套镜像枚举。
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
    /// `payload` 的结构由 rig 决定，服务端不会解析它。
    Raw {
        /// turn 标识。
        turn_id: String,
        /// rig 产出的原始 JSON。
        payload: Value,
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

/// 服务配置；字段与 `kb_svc_salvo::settings::LlmServiceConfig` 对齐。
///
/// 当前阶段 agent 只通过线协议接收该结构，因此在这里保留一个最小镜像。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmServiceConfig {
    /// provider 标识，例如 `deepseek` / `openai` / `ollama`。
    pub provider: String,

    /// 模型名。
    pub model: String,

    /// API base URL；为空表示使用 provider 默认地址。
    #[serde(default)]
    pub base_url: String,

    /// API key；为空表示尚未配置。
    #[serde(default)]
    pub api_key: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试 `Started` 事件直接携带 `abs_llm::v1::cont::Capabilities`。
    ///
    /// - 手段：构造带全部能力位的 `PluginEvent::Started`，序列化为 JSON 后
    ///   再反序列化。
    /// - 判断：JSON 中 `type` 为 `started`，能力字段为 snake_case；往返后事件与
    ///   原值完全相等，证明 agent 无需再维护一份能力结构。
    #[test]
    fn started_event_uses_abs_llm_capabilities_serde() {
        let event = PluginEvent::Started {
            turn_id: "t1".to_string(),
            model: "deepseek-chat".to_string(),
            capabilities:
                Capabilities::STREAMING +
                Capabilities::REASONING +
                Capabilities::TOOL_CALLING,
        };

        let json = serde_json::to_string(&event).expect("应当能序列化");
        assert!(json.contains(r#""type":"started""#), "实际 JSON: {json}");
        assert!(json.contains(r#""streaming":true"#), "实际 JSON: {json}");
        assert!(
            json.contains(r#""multimodal_input":false"#),
            "实际 JSON: {json}"
        );

        let parsed: PluginEvent = serde_json::from_str(&json).expect("应当能反序列化");
        assert_eq!(parsed, event);
    }

    /// 测试 `Finished` 事件直接携带 `abs_llm::v1::cont::FinishReason`。
    ///
    /// - 手段：序列化一个 `reason = Some(FinishReason::MaxTokens)` 的事件。
    /// - 判断：JSON 中 reason 为 `max_tokens`，与服务端 `wire.rs` 的 snake_case
    ///   表示一致。
    #[test]
    fn finished_event_uses_abs_llm_finish_reason_serde() {
        let event = PluginEvent::Finished {
            turn_id: "t1".to_string(),
            reason: Some(FinishReason::MaxTokens),
        };

        let json = serde_json::to_string(&event).expect("应当能序列化");
        assert!(json.contains(r#""type":"finished""#), "实际 JSON: {json}");
        assert!(
            json.contains(r#""reason":"max_tokens""#),
            "实际 JSON: {json}"
        );
    }

    /// 测试服务端 `Ask` 指令能被 agent 侧协议正确解析。
    ///
    /// - 手段：构造含完整服务配置的 JSON，反序列化为 `PluginRequest`。
    /// - 判断：`type` 为 `ask`，`service` 的 provider / model / api_key 与 JSON
    ///   一致，说明 agent 可以直接消费服务端下发的配置。
    #[test]
    fn ask_request_round_trips_with_service_config() {
        let json = serde_json::json!({
            "type": "ask",
            "turn_id": "t1",
            "service_id": "deepseek",
            "service": {
                "provider": "deepseek",
                "model": "deepseek-chat",
                "base_url": "https://api.deepseek.com",
                "api_key": "sk-test"
            },
            "question": "你好"
        });

        let request: PluginRequest = serde_json::from_value(json).expect("应当能反序列化");
        match request {
            PluginRequest::Ask {
                turn_id,
                service_id,
                service,
                question,
            } => {
                assert_eq!(turn_id, "t1");
                assert_eq!(service_id, "deepseek");
                assert_eq!(service.provider, "deepseek");
                assert_eq!(service.model, "deepseek-chat");
                assert_eq!(service.api_key, "sk-test");
                assert_eq!(question, "你好");
            }
            other => panic!("应当是 ask，实际: {other:?}"),
        }
    }
}
