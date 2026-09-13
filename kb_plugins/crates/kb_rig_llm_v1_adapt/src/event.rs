//! rig 流式事件的「可反序列化形态」与到 `abs_llm::v1` 的转换。
//!
//! # 输入是什么
//!
//! `kb_rig_llm_v1_agent` 把 rig 的**原始事件**序列化成 JSON，塞进
//! `kb_svc_salvo::wire::PluginEvent::Raw` 的 `payload` 字段送过来。
//!
//! 本模块把这些 payload 分成两类：
//!
//! | 类别 | 处理方式 |
//! | :--- | :--- |
//! | **内容分片**（文本 / reasoning / 工具调用） | 映射成 [`AdaptedEvent`]，再变成 [`attachment`] 里实现抽象层 trait 的类型 |
//! | **信封信息**（结束原因 / 用量） | 直接映射为 `abs_llm::v1::cont::FinishReason` 与 [`attachment::Usage`] |
//!
//! # 为什么在这里做「字符串 → 枚举」的判断
//!
//! rig 的流式枚举在 Rust 侧是强类型的，但**跨进程只剩 JSON**。所以判定必须落在这里：
//! 本 crate 是唯一同时了解「rig 说了什么」和「`abs_llm` 要什么」的地方。
//!
//! # 判定的依据
//!
//! 分类依据是 rig 0.42.0 的 `StreamedAssistantContent` 变体名（见
//! `src/streaming.rs`）。为了避免大小写或命名风格变化把整条链路打断，
//! 比较统一做**归一化**（转小写、`-` 与 ` ` 换成 `_`），并且对未知串采取
//! 「降级为正文」而不是「报错丢内容」的策略——丢内容对用户是更糟的失败。

use serde::{Deserialize, Serialize};

pub use super::attachment::{Capabilities, FinishReason, LogicOutput, TextDelta, ToolCall, Usage};

/// rig 原始载荷里的内容分类。
///
/// 这个枚举是**载荷里 `type` 字段的取值**经过归一化后的结果，
/// 同时也是「我们认识的类型」清单。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContentKind {
    /// 正文增量。
    Text,

    /// reasoning 增量。
    Reasoning,

    /// 工具调用。
    Tool,

    /// 其它我们认识的、但当前按正文处理的分片（例如最终答案的全文重发）。
    Other,
}

impl ContentKind {
    /// 从 payload 的 `type` 字段推断分类。
    ///
    /// 无法识别时返回 [`ContentKind::Other`]：保守地按正文处理，而不是丢弃。
    pub fn from_type_str(raw: &str) -> Self {
        match normalize_type_str(raw).as_str() {
            "text" | "answer" | "content" => Self::Text,
            "reasoning" | "reasoning_content" | "thinking" | "thought" => Self::Reasoning,
            // 复数与单数、下划线与点号写法都接受：provider 之间并不统一。
            "tool_call" | "tool_calls" | "tool" | "tools" | "function_call" | "function_calls" => {
                Self::Tool
            }
            _ => Self::Other,
        }
    }

    /// 映射为 `abs_llm::v1` 的逻辑输出分类。
    #[must_use]
    pub fn to_logic(self) -> LogicOutput {
        match self {
            Self::Text | Self::Other => LogicOutput::Answer,
            Self::Reasoning => LogicOutput::Reasoning,
            Self::Tool => LogicOutput::FunctionCall,
        }
    }
}

/// 把 rig 的类型串归一化成便于匹配的形式。
///
/// 规则依次是：
///
/// 1. 去掉首尾空白；
/// 2. 在**小写字母/数字 → 大写字母**的边界插入 `_`，即把 `TextDelta`
///    拆成 `Text_Delta`；
/// 3. 转小写，并把 `-` 与空格统一成 `_`；
/// 4. 去掉一个 `delta_` 前缀或一个 `_delta` 后缀。
///
/// 第 2 步不能省：先把 `TextDelta` 压成小写会得到 `textdelta`，
/// 那样第 4 步的前后缀匹配就永远不成立。
///
/// # 示例
///
/// ```
/// use kb_rig_llm_v1_adapt::event::normalize_type_str;
///
/// assert_eq!(normalize_type_str("TextDelta"), "text");
/// assert_eq!(normalize_type_str("delta_reasoning"), "reasoning");
/// assert_eq!(normalize_type_str("reasoning-content"), "reasoning_content");
/// ```
#[must_use]
pub fn normalize_type_str(raw: &str) -> String {
    let trimmed = raw.trim();

    // 步骤 2：camelCase → snake_case 的边界切分。
    let mut separated = String::with_capacity(trimmed.len() + 4);
    let mut previous: Option<char> = None;
    for ch in trimmed.chars() {
        if ch.is_ascii_uppercase()
            && previous.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit())
        {
            separated.push('_');
        }
        separated.push(ch);
        previous = Some(ch);
    }

    // 步骤 3：小写 + 分隔符统一。
    let normalized = separated.to_ascii_lowercase().replace(['-', ' '], "_");

    // 步骤 4：去掉一个前后缀。
    let stripped = if let Some(rest) = normalized.strip_prefix("delta_") {
        rest
    } else if let Some(rest) = normalized.strip_suffix("_delta") {
        rest
    } else {
        normalized.as_str()
    };

    stripped.to_string()
}

/// 一条原始载荷解析后的结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AdaptedEvent {
    /// 一段增量文本。
    TextDelta(TextDelta),

    /// 一次工具调用。
    ToolCall(ToolCall),

    /// 用量信息。
    Usage(Usage),

    /// 生成结束。
    Finished {
        /// 结束原因；缺失表示 provider 没有给出。
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<FinishReason>,
    },
}

impl AdaptedEvent {
    /// 从一条 `PluginEvent::Raw` 的 payload 解析。
    ///
    /// # 示例
    ///
    /// ```
    /// use kb_rig_llm_v1_adapt::event::AdaptedEvent;
    ///
    /// let payload = serde_json::json!({ "type": "text_delta", "text": "你好" });
    /// let event = AdaptedEvent::from_payload(&payload).expect("应当能解析");
    ///
    /// assert!(matches!(event, AdaptedEvent::TextDelta(_)));
    /// ```
    pub fn from_payload(payload: &serde_json::Value) -> Option<Self> {
        let object = payload.as_object()?;
        let type_str = object.get("type").and_then(|value| value.as_str())?;

        let kind = ContentKind::from_type_str(type_str);

        match kind {
            ContentKind::Text | ContentKind::Reasoning | ContentKind::Other => {
                let text = extract_text(object, kind)?;
                if text.is_empty() {
                    return None;
                }

                Some(Self::TextDelta(TextDelta::new(kind.to_logic(), text)))
            }
            ContentKind::Tool => {
                let call = extract_tool_call(object)?;
                Some(Self::ToolCall(call))
            }
        }
    }

    /// 解析用量载荷。
    ///
    /// 与 [`AdaptedEvent::from_payload`] 分开，因为 rig 把用量放在**流结束**时的
    /// 最终响应里，而不是普通的 `StreamedAssistantContent` 分片。
    pub fn usage_from_payload(payload: &serde_json::Value) -> Option<Usage> {
        let object = payload.as_object()?;

        // rig 的用法字段名沿用 OpenAI 的 `prompt_tokens` / `completion_tokens`；
        // 这里同时接受 `input_tokens` / `output_tokens`，便于换 provider 时不改代码。
        let input = read_usize(object, &["prompt_tokens", "input_tokens"]);
        let output = read_usize(object, &["completion_tokens", "output_tokens"]);
        let total = read_usize(object, &["total_tokens"]);

        let usage = Usage::new(input, output, total).with_derived_total();

        if usage.is_empty() { None } else { Some(usage) }
    }

    /// 把 rig 的结束原因字符串映射成 `abs_llm::v1` 的 [`FinishReason`]。
    ///
    /// 认不出来时返回 `None`：与其猜一个原因，不如让界面显示「已结束」。
    pub fn finish_reason_from_str(raw: &str) -> Option<FinishReason> {
        let normalized = raw.trim().to_ascii_lowercase().replace(['-', ' '], "_");

        match normalized.as_str() {
            "stop" | "completed" | "end_turn" | "finished" => Some(FinishReason::Completed),
            "length" | "max_tokens" | "max_output_tokens" => Some(FinishReason::MaxTokens),
            "cancelled" | "canceled" | "aborted" => Some(FinishReason::Cancelled),
            "tool_calls" | "tool_call" | "function_call" => Some(FinishReason::ToolCall),
            _ => None,
        }
    }
}

/// 从载荷里取出文本内容。
///
/// # 为什么按分类挑字段名
///
/// rig 的载荷形状（`rig-core` 0.42 的 `StreamedAssistantContent`，serde 用
/// `tag = "type"` 的**内部标签**）是：
///
/// - 正文分片：`{"type":"text","text":"…"}`
/// - 推理分片：`{"type":"reasoningDelta","id":"…","reasoning":"…"}`
///
/// 两者字段名不同（`text` vs `reasoning`）。**必须先看类型再挑字段**：如果无脑按
/// `text` → `content` → … 的顺序找，某个 provider 同时带了这两个字段时就会把
/// 推理内容当成正文显示。
fn extract_text(
    object: &serde_json::Map<String, serde_json::Value>,
    kind: ContentKind,
) -> Option<String> {
    let keys: &[&str] = match kind {
        ContentKind::Reasoning => &["reasoning", "reasoning_content", "thinking", "thought"],
        ContentKind::Text => &["text", "content"],
        // 未知类型时两种都试，尽量不丢内容。
        ContentKind::Other => &[
            "text",
            "content",
            "reasoning",
            "reasoning_content",
            "thinking",
        ],
        ContentKind::Tool => &[],
    };

    for key in keys {
        match object.get(*key) {
            Some(serde_json::Value::String(text)) => return Some(text.clone()),
            // 有的 provider 把内容再包一层：`{"delta": {"content": "..."}}`
            Some(serde_json::Value::Object(inner)) => {
                if let Some(text) = inner.get("content").and_then(|value| value.as_str()) {
                    return Some(text.to_string());
                }
            }
            _ => continue,
        }
    }

    // 兜底：`delta` 是个字符串时（少数 provider 的写法）。
    if let Some(serde_json::Value::String(text)) = object.get("delta") {
        return Some(text.clone());
    }

    None
}

/// 从载荷里取出一条工具调用。
fn extract_tool_call(object: &serde_json::Map<String, serde_json::Value>) -> Option<ToolCall> {
    // rig 的 ToolCall 形状：`{ id, function: { name, arguments } }`
    if let Some(function) = object.get("function").and_then(|value| value.as_object()) {
        let name = function
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let arguments = match function.get("arguments") {
            Some(serde_json::Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        let id = object
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or_default();

        return Some(ToolCall::new(id, name, arguments));
    }

    // 退化形状：字段直接摊平在顶层。
    let name = object.get("name").and_then(|value| value.as_str())?;
    let id = object
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let arguments = match object.get("arguments") {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    };

    Some(ToolCall::new(id, name, arguments))
}

/// 依次尝试多个键名读取一个非负整数。
fn read_usize(object: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<usize> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(|value| value.as_u64())
            .map(|value| value as usize)
    })
}

#[cfg(test)]
mod tests {
    use abs_llm::v1::cont::TrTextDelta;

    use super::*;

    /// 测试内容类型判定对多种命名风格都能识别。
    ///
    /// - 手段：把 `text_delta` / `TextDelta` / `reasoning-content` / `tool_calls`
    ///   等写法喂给 [`ContentKind::from_type_str`]。
    /// - 判断：大小写、`-`/`_` 混用都不影响归类；无法识别的串落到 `Other`
    ///   而不是 panic。
    #[test]
    fn content_kind_normalizes_naming_styles() {
        assert_eq!(ContentKind::from_type_str("text_delta"), ContentKind::Text);
        assert_eq!(ContentKind::from_type_str("TextDelta"), ContentKind::Text);
        assert_eq!(ContentKind::from_type_str("delta_text"), ContentKind::Text);
        assert_eq!(
            ContentKind::from_type_str("reasoning-content"),
            ContentKind::Reasoning
        );
        assert_eq!(
            ContentKind::from_type_str("ThinkingDelta"),
            ContentKind::Reasoning
        );
        assert_eq!(ContentKind::from_type_str("tool_call"), ContentKind::Tool);
        assert_eq!(ContentKind::from_type_str("tool_calls"), ContentKind::Tool);
        assert_eq!(ContentKind::from_type_str("final"), ContentKind::Other);
    }

    /// 测试文本分片被映射成 `answer` 逻辑分类。
    ///
    /// - 手段：解析一条 `{"type":"text_delta","text":"你好"}`。
    /// - 判断：得到 `TextDelta`，其 `logic()` 为 `Answer`、`text()` 为 `你好`。
    #[test]
    fn text_delta_maps_to_answer() {
        let payload = serde_json::json!({ "type": "text_delta", "text": "你好" });
        let event = AdaptedEvent::from_payload(&payload).expect("应当能解析");

        match event {
            AdaptedEvent::TextDelta(delta) => {
                assert_eq!(delta.logic_kind(), LogicOutput::Answer);
                assert_eq!(delta.text(), "你好");
            }
            other => panic!("应当是文本增量，实际: {other:?}"),
        }
    }

    /// 测试 reasoning 分片与正文分开。
    ///
    /// - 手段：解析一条 `{"type":"reasoning_delta","reasoning":"先想想"}`。
    /// - 判断：`logic()` 为 `Reasoning`，`text()` 为原文——这是界面区分样式的依据。
    #[test]
    fn reasoning_delta_maps_to_reasoning() {
        let payload = serde_json::json!({ "type": "reasoning_delta", "reasoning": "先想想" });
        let event = AdaptedEvent::from_payload(&payload).expect("应当能解析");

        match event {
            AdaptedEvent::TextDelta(delta) => {
                assert_eq!(delta.logic_kind(), LogicOutput::Reasoning);
                assert_eq!(delta.text(), "先想想");
            }
            other => panic!("应当是文本增量，实际: {other:?}"),
        }
    }

    /// 测试未知类型被降级为正文而不是丢弃。
    ///
    /// - 手段：解析一条 `{"type":"some_future_thing","text":"内容"}`。
    /// - 判断：仍然得到 `TextDelta` 且 `logic()` 为 `Answer`——宁可样式不对，
    ///   也不能把用户该看到的内容丢掉。
    #[test]
    fn unknown_type_degrades_to_answer_text() {
        let payload = serde_json::json!({ "type": "some_future_thing", "text": "内容" });
        let event = AdaptedEvent::from_payload(&payload).expect("应当能解析");

        match event {
            AdaptedEvent::TextDelta(delta) => {
                assert_eq!(delta.logic_kind(), LogicOutput::Answer)
            }
            other => panic!("应当降级为文本增量，实际: {other:?}"),
        }
    }

    /// 测试空文本分片被丢弃。
    ///
    /// - 手段：解析一条 `text` 为空串的分片。
    /// - 判断：返回 `None`，避免向界面推送无意义的空帧。
    #[test]
    fn empty_text_delta_is_dropped() {
        let payload = serde_json::json!({ "type": "text_delta", "text": "" });
        assert!(AdaptedEvent::from_payload(&payload).is_none());
    }

    /// 测试工具调用按 rig 的嵌套形状解析。
    ///
    /// - 手段：解析 `{"type":"tool_call","id":"call-1","function":{"name":...,"arguments":...}}`。
    /// - 判断：`id` / `name` / `arguments` 三者都被正确取出，且参数里的中文 JSON
    ///   保持原样（不解析、不重排）。
    #[test]
    fn tool_call_parses_nested_function_shape() {
        let payload = serde_json::json!({
            "type": "tool_call",
            "id": "call-1",
            "function": { "name": "get_weather", "arguments": "{\"city\":\"上海\"}" }
        });

        match AdaptedEvent::from_payload(&payload).expect("应当能解析") {
            AdaptedEvent::ToolCall(call) => {
                assert_eq!(call.id_ref(), "call-1");
                assert_eq!(call.name_ref(), "get_weather");
                assert_eq!(call.arguments_ref(), "{\"city\":\"上海\"}");
            }
            other => panic!("应当是工具调用，实际: {other:?}"),
        }
    }

    /// 测试用量同时接受 OpenAI 风格与通用风格的字段名。
    ///
    /// - 手段：分别解析 `prompt_tokens`/`completion_tokens` 与
    ///   `input_tokens`/`output_tokens` 两种载荷。
    /// - 判断：两种都能解析出相同的用量，并且总数在缺失时被推导出来。
    #[test]
    fn usage_accepts_both_field_naming_conventions() {
        let openai_style = serde_json::json!({
            "prompt_tokens": 12,
            "completion_tokens": 5
        });
        let generic_style = serde_json::json!({
            "input_tokens": 12,
            "output_tokens": 5
        });

        let a = AdaptedEvent::usage_from_payload(&openai_style).expect("应当能解析");
        let b = AdaptedEvent::usage_from_payload(&generic_style).expect("应当能解析");

        assert_eq!(a.total(), Some(17));
        assert_eq!(b.total(), Some(17));
        assert_eq!(a.input(), b.input());
    }

    /// 测试没有任何计数的用量载荷被丢弃。
    ///
    /// - 手段：解析一个空对象。
    /// - 判断：返回 `None`，避免向界面推送「token：未知」这种空信息。
    #[test]
    fn empty_usage_payload_is_dropped() {
        assert!(AdaptedEvent::usage_from_payload(&serde_json::json!({})).is_none());
    }

    /// 测试结束原因映射覆盖常见取值，认不出时返回 `None`。
    ///
    /// - 手段：映射 `stop` / `length` / `tool_calls` / `cancelled` 与一个陌生串。
    /// - 判断：前四者映射到对应变体，陌生串返回 `None`。
    #[test]
    fn finish_reason_maps_common_values() {
        assert_eq!(
            AdaptedEvent::finish_reason_from_str("stop"),
            Some(FinishReason::Completed)
        );
        assert_eq!(
            AdaptedEvent::finish_reason_from_str("length"),
            Some(FinishReason::MaxTokens)
        );
        assert_eq!(
            AdaptedEvent::finish_reason_from_str("tool_calls"),
            Some(FinishReason::ToolCall)
        );
        assert_eq!(
            AdaptedEvent::finish_reason_from_str("Cancelled"),
            Some(FinishReason::Cancelled)
        );
        assert_eq!(AdaptedEvent::finish_reason_from_str("mystery"), None);
    }

    /// 测试解析结果能直接序列化成浏览器要的 `abs_llm` 形状。
    ///
    /// - 手段：把 `AdaptedEvent::TextDelta` 序列化为 JSON。
    /// - 判断：得到 `{"kind":"text_delta","logic":"reasoning","text":"…"}`——
    ///   即 `kb_svc_salvo` 不需要再加工就能转发。
    #[test]
    fn adapted_event_serializes_to_wire_shape() {
        let event = AdaptedEvent::TextDelta(TextDelta::new(LogicOutput::Reasoning, "想了想"));
        let json = serde_json::to_string(&event).expect("应当能序列化");

        assert!(json.contains(r#""kind":"text_delta""#), "实际 JSON: {json}");
        assert!(json.contains(r#""logic":"reasoning""#), "实际 JSON: {json}");
        assert!(json.contains(r#""text":"想了想""#), "实际 JSON: {json}");
    }

    /// 测试类型串归一化的每一步都能单独观察。
    ///
    /// - 手段：把四种写法喂给 [`normalize_type_str`]。
    /// - 判断：大小写被压平、`-` 变成 `_`、`delta_` 前缀与 `_delta` 后缀各去掉一个；
    ///   把归一化单独拆出来测，是为了让「匹配失败」这类问题一眼能定位到是哪一步。
    #[test]
    fn normalize_type_str_strips_delta_affixes() {
        assert_eq!(normalize_type_str("TextDelta"), "text");
        assert_eq!(normalize_type_str("text_delta"), "text");
        assert_eq!(normalize_type_str("delta_text"), "text");
        assert_eq!(normalize_type_str("reasoning-content"), "reasoning_content");
        assert_eq!(normalize_type_str("  Thinking  "), "thinking");
        assert_eq!(normalize_type_str("final"), "final");
    }

    /// 测试 rig 真实载荷形状能被正确区分（正文 vs 推理）。
    ///
    /// - 手段：喂入 `rig-core` 0.42 的两种真实分片形状——
    ///   `{"type":"text","text":…}` 与
    ///   `{"type":"reasoningDelta","id":…,"reasoning":…}`。
    /// - 判断：前者落到 `Answer`、后者落到 `Reasoning`，内容各自取对字段；
    ///   这能挡住「两者字段名不同、按错误顺序找字段」这类 bug。
    #[test]
    fn rig_0_42_payload_shapes_map_to_correct_logic() {
        let text = serde_json::json!({ "type": "text", "text": "正式回答" });
        let reasoning = serde_json::json!({
            "type": "reasoningDelta",
            "id": "r1",
            "reasoning": "推理内容"
        });

        match AdaptedEvent::from_payload(&text).expect("应当能解析") {
            AdaptedEvent::TextDelta(delta) => {
                assert_eq!(delta.logic_kind(), LogicOutput::Answer);
                assert_eq!(delta.text_ref(), "正式回答");
            }
            other => panic!("应当是文本增量，实际: {other:?}"),
        }

        match AdaptedEvent::from_payload(&reasoning).expect("应当能解析") {
            AdaptedEvent::TextDelta(delta) => {
                assert_eq!(delta.logic_kind(), LogicOutput::Reasoning);
                assert_eq!(delta.text_ref(), "推理内容");
            }
            other => panic!("应当是文本增量，实际: {other:?}"),
        }
    }

    /// 测试同时带 `text` 与 `reasoning` 的载荷按类型取对字段。
    ///
    /// - 手段：构造一条 `type = reasoningDelta` 但同时含 `text` 的载荷。
    /// - 判断：取到的是 `reasoning` 字段而不是 `text`——顺序错了就会串味。
    #[test]
    fn reasoning_type_prefers_reasoning_field() {
        let payload = serde_json::json!({
            "type": "reasoningDelta",
            "text": "不该被取用",
            "reasoning": "应当被取用"
        });

        match AdaptedEvent::from_payload(&payload).expect("应当能解析") {
            AdaptedEvent::TextDelta(delta) => assert_eq!(delta.text_ref(), "应当被取用"),
            other => panic!("应当是文本增量，实际: {other:?}"),
        }
    }
}
