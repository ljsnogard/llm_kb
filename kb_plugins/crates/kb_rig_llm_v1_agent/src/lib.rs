//! # kb_rig_llm_v1_agent
//!
//! 使用 rig 直接连接 LLM provider 的插件进程。
//!
//! 本 crate 负责维护对话上下文，并把 rig 的**原始数据**通过插件线协议上报给
//! `kb_svc_salvo`；语义转换发生在服务端进程内的 `kb_rig_llm_v1_adapt`，而不是
//! 这里（见 `dev-notes.md` §2.2 / §2.3）。
//!
//! [`protocol`] 定义 agent 侧的信封级线协议。`abs_llm` 临时引入 serde 后
//! （`dev-notes.md` §2.5），`Capabilities` / `FinishReason` 等语义字段直接复用
//! `abs_llm::v1` 类型，不再在 agent 里维护镜像枚举。

pub mod protocol;
