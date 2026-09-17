//! # kb_rig_llm_v1_adapt
//!
//! 把 rig 产出的原始数据转换成 `abs_llm::v1` 的数据。
//!
//! 依据 `dev-notes/llm_kb-20260917-1655.md` §1.1，本 crate 承担两件事：
//!
//! 1. **语义映射**：rig 的流式分片 → `abs_llm::v1` 的 `LogicOutput` /
//!    `FinishReason` / `TrUsage` 等概念；
//! 2. **线上的具体表示**：`abs_llm` 现在为纯语义类型提供基础 serde 派生
//!    （临时决策，见 §2.5），但具体载荷（`TextDelta` / `ToolCall` / `Usage` /
//!    `AdaptedEvent`）的 JSON 形状、缺省字段与原始 rig payload 解析仍由本 crate
//!    决定。
//!
//! # 转换发生在哪里
//!
//! `kb_rig_llm_v1_agent` 把 rig 的原始数据**原样**发给 `kb_svc_salvo`，转换发生
//! 在 `kb_svc_salvo` 进程内（§2.2 / §2.3）：
//!
//! ```text
//! agent ──rig 原始 JSON──► kb_svc_salvo ──本 crate──► abs_llm::v1 形状 ──► 浏览器
//! ```
//!
//! # 两类公开类型
//!
//! | 模块 | 用途 |
//! | :--- | :--- |
//! | [`attachment`] | 实现 `abs_llm::v1` 那些 trait 的具体类型，可被抽象层的泛型代码消费 |
//! | [`event`] | rig 流式事件的「可反序列化形态」，以及到上述类型的转换 |
//!
//! # 示例
//!
//! ```
//! use kb_rig_llm_v1_adapt::{attachment::TextDelta, event::LogicOutput};
//!
//! let delta = TextDelta::new(LogicOutput::Reasoning, "先想想");
//!
//! // 既有强类型视图……
//! assert_eq!(delta.logic_kind(), LogicOutput::Reasoning);
//! assert_eq!(delta.text_ref(), "先想想");
//!
//! // ……也可以直接序列化成线上表示。
//! let json = serde_json::to_string(&delta).unwrap();
//! assert!(json.contains(r#""logic":"reasoning""#));
//! ```

pub mod attachment;
pub mod event;
