//! LLM 服务配置，以及 API key 的更新语义。
//!
//! # API key 的方向性
//!
//! 明文 key **只有一个方向会出现**：客户端 → `kb_core`（
//! [`ApiKeyUpdate::Set`]）。反方向（`kb_core` → 客户端）永远只有掩码，
//! 因为 [`ServiceSummary`] 里根本没有明文字段。需要明文的是**插件**，
//! 由 `kb_core` 在 `v1::plugin` 那一侧交给它。

use serde::{Deserialize, Serialize};

use super::ids_::ServiceId;

/// API key 的掩码。
///
/// 与 `kb_svc_salvo::settings::MASKED_KEY` 以及桌面客户端的 `kMaskedApiKey`
/// 保持同一个字符串（U+2022 重复 8 次）。
///
/// **兼容性要求**：服务端在收到"把掩码原样回传"的旧式客户端时，
/// 必须把它解释为 [`ApiKeyUpdate::Keep`]，而不是把掩码写进配置。
pub const MASKED_API_KEY: &str = "••••••••";

/// API key 的更新语义。
///
/// 客户端拿到的服务配置里 key 永远是掩码，因此"编辑服务"这个动作必须能表达
/// "我没改 key"与"我要清空 key"两件事，不能只靠一个字符串。
///
/// # 示例
///
/// ```
/// use abs_kb_svc::v1::desktop::ApiKeyUpdate;
///
/// let keep = ApiKeyUpdate::Keep;
/// let set = ApiKeyUpdate::Set {
///     value: "sk-test".to_string(),
/// };
/// let clear = ApiKeyUpdate::Clear;
///
/// // 三者是不同的语义，序列化后也能区分
/// assert_ne!(
///     serde_json::to_string(&keep).unwrap(),
///     serde_json::to_string(&set).unwrap()
/// );
/// assert_ne!(
///     serde_json::to_string(&set).unwrap(),
///     serde_json::to_string(&clear).unwrap()
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApiKeyUpdate {
    /// 保持原有 key 不变。
    Keep,

    /// 设置为新值。
    Set {
        /// 新的 API key 明文（仅在客户端 → 服务端方向出现）。
        value: String,
    },

    /// 清空已有 key。
    Clear,
}

/// 一个 LLM 服务的对外摘要。
///
/// **注意**：这里只出现掩码，明文 key 永不出现在任何服务端 → 客户端的方向上。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSummary {
    /// 服务标识（用户取的名字）。
    pub service_id: ServiceId,

    /// provider 标识，例如 `deepseek`。
    pub provider: String,

    /// 模型名，例如 `deepseek-chat`。
    pub model: String,

    /// API base URL；空表示用 provider 默认地址。
    #[serde(default)]
    pub base_url: String,

    /// 是否为当前生效的服务。
    pub active: bool,

    /// 是否已经配置了 API key。
    pub has_api_key: bool,

    /// 掩码后的 key，仅供界面回显；未配置时为空串。
    pub api_key_masked: String,
}

/// 服务列表及其生效项。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceList {
    /// 已配置的服务。
    pub services: Vec<ServiceSummary>,

    /// 当前生效的服务标识；没有可用服务时为 `None`。
    #[serde(default)]
    pub active_service: Option<ServiceId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试三种 key 更新语义互不混淆。
    ///
    /// - 手段：把 `Keep` / `Set` / `Clear` 分别序列化。
    /// - 判断：三者 JSON 两两不同，且 `Set` 携带 `value` 字段；
    ///   这直接支撑"界面回显掩码时不能把掩码写进配置"这一约定。
    #[test]
    fn api_key_update_variants_are_distinguishable_() {
        let keep = serde_json::to_string(&ApiKeyUpdate::Keep).expect("应当能序列化");
        let set = serde_json::to_string(&ApiKeyUpdate::Set {
            value: MASKED_API_KEY.to_string(),
        })
        .expect("应当能序列化");
        let clear = serde_json::to_string(&ApiKeyUpdate::Clear).expect("应当能序列化");

        assert_ne!(keep, set);
        assert_ne!(set, clear);
        assert_ne!(keep, clear);
        assert!(set.contains("value"), "实际 JSON: {set}");
    }

    /// 测试服务摘要只携带掩码，不携带明文 key。
    ///
    /// - 手段：构造一个带掩码的 `ServiceSummary` 并序列化。
    /// - 判断：JSON 中出现掩码字符串，且不含 `"api_key":` 这个明文字段名——
    ///   即"服务端永不把明文 key 回传客户端"这一约定在类型层面就成立。
    #[test]
    fn service_summary_never_carries_plaintext_key_() {
        let summary = ServiceSummary {
            service_id: ServiceId::new("deepseek"),
            provider: "deepseek".to_string(),
            model: "deepseek-chat".to_string(),
            base_url: String::new(),
            active: true,
            has_api_key: true,
            api_key_masked: MASKED_API_KEY.to_string(),
        };

        let json = serde_json::to_string(&summary).expect("应当能序列化");
        assert!(json.contains(MASKED_API_KEY), "实际 JSON: {json}");
        assert!(!json.contains(r#""api_key":"#), "不应出现明文字段: {json}");
    }
}
