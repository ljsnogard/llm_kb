//! 一条消息（[`Turn`]）及其组成部分：状态、工具调用、用量、提示。
//!
//! 这里的字段与桌面客户端的 `ChatTurn` 一一对应，因此客户端可以在收到
//! 增量与结束时直接更新界面模型，不需要额外的映射层。

use abs_llm::v1::cont::Role;
use serde::{Deserialize, Serialize};

use super::ids_::TurnId;

/// 一条消息的生成状态。
///
/// 对应桌面客户端的 `ChatTurnState`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnState {
    /// 正在增量生成。
    Streaming,

    /// 已结束（正常结束或被取消）。
    Done,

    /// 以错误结束。
    Failed,
}

/// 一次工具调用的记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallRecord {
    /// 工具调用标识（由 provider 给出）。
    pub call_id: String,

    /// 工具名。
    pub name: String,

    /// 参数（原始 JSON 文本，服务端不解释它）。
    pub arguments: String,
}

/// Token 用量。
///
/// 三个计数都可缺省：provider 不保证给出精确值。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    /// 输入 token 数。
    #[serde(default)]
    pub input_tokens: Option<u64>,

    /// 输出 token 数。
    #[serde(default)]
    pub output_tokens: Option<u64>,

    /// 总 token 数。
    #[serde(default)]
    pub total_tokens: Option<u64>,
}

/// 挂在一条消息上的提示（错误说明或普通提示）。
///
/// 对应桌面客户端的 `ChatNotice`。界面把它渲染成消息下方的一行说明文字，
/// 因此它既承载错误也承载"非错误的说明"。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    /// 提示正文。
    pub message: String,

    /// 是否为错误提示（决定界面配色与图标）。
    pub is_error: bool,
}

/// 一条消息（一问或一答）。
///
/// 对应桌面客户端的 `ChatTurn`。历史消息通过
/// [`Request::GetSession`](crate::v1::desktop::Request::GetSession) 一次性取回，
/// 生成过程中的消息通过 [`Event`](crate::v1::desktop::Event) 增量更新。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    /// 回合标识；客户端与 `kb_core` 都以此关联一轮生成。
    pub turn_id: TurnId,

    /// 说话人。
    ///
    /// 直接复用 `abs_llm::v1::cont::Role`（`system` / `user` / `assistant` / `tool`），
    /// 不再像旧的 `kb_svc_salvo::wire` 那样镜像一份枚举。
    pub role: Role,

    /// 回答正文。
    pub text: String,

    /// 推理正文（与正文分开保存，界面可以选择样式）。
    pub reasoning: String,

    /// 生成状态。
    pub state: TurnState,

    /// 本轮的工具调用。
    #[serde(default)]
    pub tool_calls: Vec<ToolCallRecord>,

    /// token 用量。
    #[serde(default)]
    pub usage: Option<TokenUsage>,

    /// 错误或说明。
    #[serde(default)]
    pub notice: Option<Notice>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试消息的字段名与取值都与客户端模型兼容。
    ///
    /// - 手段：构造一条带提示的助手消息并序列化。
    /// - 判断：JSON 中 `turn_id` 是裸字符串、`role` 为 `assistant`、
    ///   `state` 为 `done`；空的 `tool_calls` 编码为 `[]`、缺省的 `usage`
    ///   编码为 `null`（不省略，见 `optional_fields_are_always_encoded_` 的理由）——
    ///   这些正是客户端 `ChatTurn.fromJson` 读取的字段。
    #[test]
    fn turn_matches_client_model_() {
        let turn = Turn {
            turn_id: TurnId::new("t-1"),
            role: Role::Assistant,
            text: "你好".to_string(),
            reasoning: String::new(),
            state: TurnState::Done,
            tool_calls: Vec::new(),
            usage: None,
            notice: Some(Notice {
                message: "插件离线".to_string(),
                is_error: true,
            }),
        };

        let json = serde_json::to_string(&turn).expect("应当能序列化");
        assert!(json.contains(r#""turn_id":"t-1""#), "实际 JSON: {json}");
        assert!(json.contains(r#""role":"assistant""#), "实际 JSON: {json}");
        assert!(json.contains(r#""state":"done""#), "实际 JSON: {json}");
        assert!(json.contains(r#""is_error":true"#), "实际 JSON: {json}");
        assert!(json.contains(r#""tool_calls":[]"#), "实际 JSON: {json}");
        assert!(json.contains(r#""usage":null"#), "实际 JSON: {json}");

        let parsed: Turn = serde_json::from_str(&json).expect("应当能反序列化");
        assert_eq!(parsed, turn);
    }

    /// 测试用量三个计数缺省时仍被完整编码，且互不影响。
    ///
    /// - 手段：只填 `total_tokens` 构造 `TokenUsage`，序列化后反序列化。
    /// - 判断：JSON 中 `input_tokens` / `output_tokens` 为 `null`（而非被省略）；
    ///   解析回来两者为 `None`、`total_tokens` 保持 12。
    #[test]
    fn token_usage_keeps_null_counters_() {
        let usage = TokenUsage {
            input_tokens: None,
            output_tokens: None,
            total_tokens: Some(12),
        };

        let json = serde_json::to_string(&usage).expect("应当能序列化");
        assert!(json.contains(r#""input_tokens":null"#), "实际 JSON: {json}");

        let parsed: TokenUsage = serde_json::from_str(&json).expect("应当能反序列化");
        assert_eq!(parsed, usage);
    }
}
