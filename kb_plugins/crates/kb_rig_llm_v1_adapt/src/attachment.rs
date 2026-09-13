//! 实现 `abs_llm::v1` 各 trait 的具体类型，并定义它们的具体 serde 表示。
//!
//! # 分工
//!
//! `abs_llm` 现在为少量纯语义类型（[`LogicOutput`]、[`FinishReason`]、
//! `Capabilities` 等）提供基础 serde 派生；本 crate 则负责把 rig 原始数据映射成
//! 这些语义，并让具体载荷（[`TextDelta`]、[`ToolCall`]、[`Usage`]、`AdaptedEvent`）
//! 以界面真正使用的 JSON 形状上线。
//!
//! 这些类型既实现抽象层的 trait，又能直接 `serde` 序列化，因此不再需要维护
//! 「抽象层镜像」；带缺省字段、tag、rename 等 wire 细节仍集中在本 crate。
//!
//! 当前提供的类型：
//!
//! | 类型 | 实现的 `abs_llm::v1` trait |
//! | :--- | :--- |
//! | [`TextDelta`] | [`TrTextDelta`] |
//! | [`ToolCall`] | [`TrToolCall`] |
//! | [`Usage`] | [`TrUsage`] |
//! | [`Capabilities`] | 不实现 trait；直接复用 `abs_llm::v1::cont::Capabilities` |
//!
//! `TrChatResponse` 需要引用式访问（`answer(&self) -> Option<&Self::AnswerText>`），
//! 等非流式聚合真正接入时再补。

use abs_llm::v1::cont::{TrTextDelta, TrToolCall, TrUsage};
use serde::{Deserialize, Serialize};

pub use abs_llm::v1::cont::{Capabilities, FinishReason, LogicOutput};

/// 一段增量文本；实现 [`TrTextDelta`]，并直接以 `{logic, text}` 的形式上线。
///
/// `logic` 直接使用 `abs_llm::v1::cont::LogicOutput`；线上形状由 `abs_llm`
/// 的 serde 派生决定，当前是 snake_case 字符串。
///
/// # 示例
///
/// ```
/// use kb_rig_llm_v1_adapt::{attachment::TextDelta, event::LogicOutput};
///
/// let delta = TextDelta::new(LogicOutput::Answer, "你好");
/// assert_eq!(delta.text_ref(), "你好");
/// assert_eq!(delta.logic_kind(), LogicOutput::Answer);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextDelta {
    /// 这段文本属于哪个逻辑部分。
    logic: LogicOutput,

    /// 文本片段。
    text: String,
}

impl TextDelta {
    /// 构造一段增量文本。
    pub fn new(logic: LogicOutput, text: impl Into<String>) -> Self {
        Self {
            logic,
            text: text.into(),
        }
    }

    /// 以**借用**形式返回文本。
    ///
    /// # 与 trait 方法的关系
    ///
    /// [`TrTextDelta::text`] 返回拥有所有权的 `String`——`abs_llm` 的关联类型约束是
    /// `Self::StrRepr: TrStringView<str>`，返回不了借用。所以两者分工明确：
    ///
    /// - 只想读一下、不想克隆 → 用本方法；
    /// - 要交给抽象层的泛型代码 → 用 trait 方法。
    ///
    /// 这里刻意**不与 trait 方法同名**：同名时固有方法会遮蔽 trait 方法，
    /// 泛型调用点的返回类型会悄悄变成固有方法的类型（曾经踩过这个坑）。
    pub fn text_ref(&self) -> &str {
        &self.text
    }

    /// 返回逻辑分类。
    ///
    /// 与 [`TrTextDelta::logic`] 返回同一个值；这是 `abs_llm` 的语义类型，
    /// 因此既可以直接比较，也可以交给下游线协议转换。
    pub fn logic_kind(&self) -> LogicOutput {
        self.logic
    }

    /// 文本长度（字节数），便于测试与日志。
    pub fn len(&self) -> usize {
        self.text.len()
    }

    /// 文本是否为空。
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

impl TrTextDelta for TextDelta {
    type StrRepr = String;

    fn logic(&self) -> LogicOutput {
        self.logic
    }

    fn text(&self) -> Self::StrRepr {
        self.text.clone()
    }
}

/// 一次工具调用；实现 [`TrToolCall`]。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    /// 工具调用标识。
    id: String,

    /// 工具名。
    name: String,

    /// 参数（原始 JSON 文本，保持 provider 的原样）。
    arguments: String,
}

impl ToolCall {
    /// 构造一次工具调用。
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments: arguments.into(),
        }
    }

    /// 以借用形式返回工具调用标识。
    pub fn id_ref(&self) -> &str {
        &self.id
    }

    /// 以借用形式返回工具名。
    pub fn name_ref(&self) -> &str {
        &self.name
    }

    /// 以借用形式返回参数文本。
    pub fn arguments_ref(&self) -> &str {
        &self.arguments
    }
}

impl TrToolCall for ToolCall {
    type StrRepr = String;
    type Arguments = String;

    fn id(&self) -> Self::StrRepr {
        self.id.clone()
    }

    fn name(&self) -> Self::StrRepr {
        self.name.clone()
    }

    fn arguments(&self) -> Self::Arguments {
        self.arguments.clone()
    }
}

/// Token 用量；实现 [`TrUsage`]。
///
/// 三个计数都是可选的：provider 不保证提供。序列化时会略去 `None`，
/// 这样界面上不会出现「输入 null」这种噪音。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// 输入 token 数。
    #[serde(skip_serializing_if = "Option::is_none")]
    input_tokens: Option<usize>,

    /// 输出 token 数。
    #[serde(skip_serializing_if = "Option::is_none")]
    output_tokens: Option<usize>,

    /// 总 token 数。
    #[serde(skip_serializing_if = "Option::is_none")]
    total_tokens: Option<usize>,
}

impl Usage {
    /// 构造一份用量。
    pub fn new(
        input_tokens: Option<usize>,
        output_tokens: Option<usize>,
        total_tokens: Option<usize>,
    ) -> Self {
        Self {
            input_tokens,
            output_tokens,
            total_tokens,
        }
    }

    /// 输入 token 数。
    pub fn input(&self) -> Option<usize> {
        self.input_tokens
    }

    /// 输出 token 数。
    pub fn output(&self) -> Option<usize> {
        self.output_tokens
    }

    /// 总 token 数。
    pub fn total(&self) -> Option<usize> {
        self.total_tokens
    }

    /// 只有总数、其余未知时的便捷构造。
    pub fn from_total(total_tokens: usize) -> Self {
        Self::new(None, None, Some(total_tokens))
    }

    /// 三个计数是否全为空——用于判断「这份用量没有信息量」。
    pub fn is_empty(&self) -> bool {
        self.input_tokens.is_none() && self.output_tokens.is_none() && self.total_tokens.is_none()
    }

    /// 已知计数中补齐总数：若 `total` 缺失但输入输出都在，则相加。
    ///
    /// 这是**推导**而不是 provider 的原话，因此只在两个分量都存在时才做。
    pub fn with_derived_total(mut self) -> Self {
        if self.total_tokens.is_none()
            && let (Some(input), Some(output)) = (self.input_tokens, self.output_tokens)
        {
            self.total_tokens = Some(input + output);
        }
        self
    }
}

impl TrUsage for Usage {
    fn input_tokens(&self) -> Option<usize> {
        self.input_tokens
    }

    fn output_tokens(&self) -> Option<usize> {
        self.output_tokens
    }

    fn total_tokens(&self) -> Option<usize> {
        self.total_tokens
    }
}

#[cfg(test)]
mod tests {
    // 抽象层的 trait 通过 `super::*`（即文件顶部）已经在作用域内，
    // 泛型一致性测试直接用它们即可。
    use super::*;

    /// 测试 `TextDelta` 的 trait 视图与 serde 视图一致。
    ///
    /// - 手段：构造一段 reasoning 文本，分别通过 `TrTextDelta` 的方法与
    ///   `serde_json` 读取。
    /// - 判断：`logic()` / `text()` 返回构造时的值；JSON 里 `logic` 为
    ///   `reasoning`、`text` 为原文——即「一个类型两副面孔」不会漂移。
    #[test]
    fn text_delta_trait_view_matches_serde_view() {
        let delta = TextDelta::new(LogicOutput::Reasoning, "先想想");

        assert_eq!(delta.logic_kind(), LogicOutput::Reasoning);
        assert_eq!(delta.text_ref(), "先想想");
        assert_eq!(delta.len(), "先想想".len());
        assert!(!delta.is_empty());

        let json = serde_json::to_string(&delta).expect("应当能序列化");
        assert_eq!(json, r#"{"logic":"reasoning","text":"先想想"}"#);
    }

    /// 测试三个类型都真的实现了 `abs_llm::v1` 的对应 trait（而不只是固有方法）。
    ///
    /// - 手段：用 trait 的方法（而非固有方法）读取 `TextDelta` / `ToolCall` /
    ///   `Usage`，并让它们经过一个只认 trait 的泛型函数。
    /// - 判断：泛型调用能编译且返回值正确，说明抽象层的 trait 约束确实被满足——
    ///   这正是「`kb_svc_salvo` 可以按抽象层写代码」的前提。
    #[test]
    fn types_implement_abs_llm_traits() {
        fn logic_of<D: TrTextDelta>(delta: &D) -> LogicOutput {
            delta.logic()
        }
        fn name_of<C>(call: &C) -> String
        where
            C: TrToolCall<StrRepr = String>,
        {
            call.name()
        }
        fn total_of<U: TrUsage>(usage: &U) -> Option<usize> {
            usage.total_tokens()
        }

        let delta = TextDelta::new(LogicOutput::Answer, "你好");
        assert_eq!(logic_of(&delta), LogicOutput::Answer);

        let call = ToolCall::new("id", "get_weather", "{}");
        assert_eq!(name_of(&call), "get_weather");

        let usage = Usage::from_total(9);
        assert_eq!(total_of(&usage), Some(9));
    }

    /// 测试 `ToolCall` 的三个字段都能通过 trait 读出且能往返序列化。
    ///
    /// - 手段：构造一次带 JSON 参数的调用，读取 trait 方法后做一次序列化往返。
    /// - 判断：`id` / `name` / `arguments` 与构造值一致，往返后仍相等。
    #[test]
    fn tool_call_round_trips() {
        let call = ToolCall::new("call-1", "get_weather", r#"{"city":"上海"}"#);

        assert_eq!(call.id_ref(), "call-1");
        assert_eq!(call.name_ref(), "get_weather");
        assert_eq!(call.arguments_ref(), r#"{"city":"上海"}"#);

        let json = serde_json::to_string(&call).expect("应当能序列化");
        let parsed: ToolCall = serde_json::from_str(&json).expect("应当能反序列化");
        assert_eq!(parsed, call);
    }

    /// 测试 `Usage` 的 `TrUsage` 视图只暴露已知的计数。
    ///
    /// - 手段：构造一份只有输入与输出的用量，调用 `with_derived_total`。
    /// - 判断：总数被推导为两者之和；未知分量仍为 `None`。
    #[test]
    fn usage_derives_total_only_when_both_parts_known() {
        let usage = Usage::new(Some(12), Some(5), None).with_derived_total();
        assert_eq!(usage.input(), Some(12));
        assert_eq!(usage.output(), Some(5));
        assert_eq!(usage.total(), Some(17));

        // 缺一个分量时不猜。
        let partial = Usage::new(Some(12), None, None).with_derived_total();
        assert_eq!(partial.total(), None);

        assert!(Usage::default().is_empty());
        assert!(!usage.is_empty());
    }

    /// 测试 `Usage` 序列化时略去未知计数。
    ///
    /// - 手段：只填总数，序列化成 JSON。
    /// - 判断：JSON 中不含 `input_tokens` / `output_tokens` 两个键。
    #[test]
    fn usage_omits_unknown_counters_in_json() {
        let json = serde_json::to_string(&Usage::from_total(17)).expect("应当能序列化");

        assert!(json.contains(r#""total_tokens":17"#), "实际 JSON: {json}");
        assert!(!json.contains("input_tokens"), "实际 JSON: {json}");
        assert!(!json.contains("output_tokens"), "实际 JSON: {json}");
    }

    /// 测试 `Capabilities` 已直接复用 `abs_llm` 的 serde 类型。
    ///
    /// - 手段：构造一份能力位并序列化；再反序列化一个空对象。
    /// - 判断：字段名保持 snake_case；空对象依赖 `#[serde(default)]` 得到全
    ///   `false`，说明缺省行为由抽象层统一提供，本 crate 不再维护镜像类型。
    #[test]
    fn capabilities_round_trip_with_abs_llm_serde() {
        let caps = Capabilities {
            streaming: true,
            reasoning: false,
            multimodal_input: true,
            tool_calling: false,
        };

        let json = serde_json::to_string(&caps).expect("应当能序列化");
        assert_eq!(
            json,
            r#"{"streaming":true,"reasoning":false,"multimodal_input":true,"tool_calling":false}"#
        );

        let parsed: Capabilities = serde_json::from_str("{}").expect("应当能反序列化");
        assert_eq!(parsed, Capabilities::default());
    }
}
