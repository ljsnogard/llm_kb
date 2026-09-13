//! 会话中枢：把浏览器的请求转发给插件，再把插件的事件广播给所有浏览器。
//!
//! # 职责
//!
//! - 持有当前生效的服务标识与「正在进行的 turn」的**极简**状态；
//! - 维护插件连接（可能有多个，通常是零个或一个）的写出通道；
//! - 用 [`tokio::sync::broadcast`] 把助手输出广播给所有已打开的页面，
//!   因此多个标签页能同时看到同一轮输出。
//!
//! # 内存态策略
//!
//! 当前阶段刻意**不保存对话历史**：一轮结束后只保留「上一轮的 turn 标识」用于
//! 状态判断，历史由浏览器侧暂存。这符合 `dev-notes.md` §9 的 MVP 范围，
//! 真正的历史与会话持久化留到后续阶段。

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use kb_rig_llm_v1_adapt::event::AdaptedEvent;

use crate::{
    settings::{LlmServiceConfig, SettingsStore},
    wire::{
        ClientMessage, ErrorCode, FinishReason, LogicOutput, PluginEvent, PluginRequest,
        ServerMessage, Usage,
    },
};

/// 服务端版本，随 `ready` 帧下发，便于排查前后端不一致。
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 广播通道的容量。
///
/// 单个 delta 帧很小，256 条足以覆盖一次渲染卡顿；超出后订阅者会收到
/// `RecvError::Lagged`，由 WS 任务转成一条错误提示。
const BROADCAST_CAPACITY: usize = 256;

/// 每个插件连接的写出队列容量。
const PLUGIN_CHANNEL_CAPACITY: usize = 64;

/// 中枢处理请求失败时的错误。
///
/// 它不是线协议类型：由调用方（WebSocket 任务）转成 [`ServerMessage::Error`] 再发出，
/// 这样可以避免中枢直接构造「发给浏览器」的消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubError {
    /// 相关的 turn（若错误与某一轮生成有关）。
    pub turn_id: Option<String>,

    /// 错误类别。
    pub code: ErrorCode,

    /// 面向用户的说明。
    pub message: String,
}

impl HubError {
    /// 构造一条与具体 turn 无关的错误。
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            turn_id: None,
            code,
            message: message.into(),
        }
    }

    /// 构造一条与具体 turn 相关的错误。
    ///
    /// 保留 `turn_id` 是为了让界面能把错误挂到对应的那一轮上，而不是笼统地弹一条提示。
    pub fn for_turn(
        turn_id: impl Into<String>,
        code: ErrorCode,
        message: impl Into<String>,
    ) -> Self {
        Self {
            turn_id: Some(turn_id.into()),
            code,
            message: message.into(),
        }
    }

    /// 转换成本次连接直接回给浏览器的消息。
    pub fn into_message(self) -> ServerMessage {
        ServerMessage::Error {
            turn_id: self.turn_id,
            code: self.code,
            message: self.message,
        }
    }
}

/// 把 adapter 的逻辑分类映射成线协议枚举。
///
/// 两个枚举的变体一一对应，这里显式列出而不是用 `From` 派生，
/// 是为了将来任一侧新增变体时编译器会在这里报错，而不是悄悄漏掉。
fn to_wire_logic(value: kb_rig_llm_v1_adapt::event::LogicOutput) -> LogicOutput {
    use kb_rig_llm_v1_adapt::event::LogicOutput as Adapted;
    match value {
        Adapted::Answer => LogicOutput::Answer,
        Adapted::Reasoning => LogicOutput::Reasoning,
        Adapted::FunctionCall => LogicOutput::FunctionCall,
        Adapted::DynamicSearchCall => LogicOutput::DynamicSearchCall,
        Adapted::StaticSearchCall => LogicOutput::StaticSearchCall,
    }
}

/// 把 adapter 的结束原因映射成线协议枚举。
fn to_wire_finish_reason(value: kb_rig_llm_v1_adapt::event::FinishReason) -> FinishReason {
    use kb_rig_llm_v1_adapt::event::FinishReason as Adapted;
    match value {
        Adapted::Completed => FinishReason::Completed,
        Adapted::MaxTokens => FinishReason::MaxTokens,
        Adapted::Cancelled => FinishReason::Cancelled,
        Adapted::ToolCall => FinishReason::ToolCall,
        Adapted::Other => FinishReason::Other,
    }
}

/// 当前活动的一轮生成。
#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveTurn {
    /// turn 标识。
    turn_id: String,
    /// 使用的服务标识。
    service_id: String,
    /// 是否已经收到插件的 `started`（用于判断重复的 started 是否可疑）。
    started: bool,
}

/// 应用共享状态。
#[derive(Debug, Clone)]
pub struct AppState(Arc<Inner>);

#[derive(Debug)]
struct Inner {
    /// 设置存储与缓存。
    store: SettingsStore,
    /// 当前生效的服务标识。
    active_service: RwLock<Option<String>>,
    /// 正在进行的 turn。
    active_turn: RwLock<Option<ActiveTurn>>,
    /// 插件连接：连接标识 → 写出通道。
    plugins: RwLock<HashMap<Uuid, mpsc::Sender<PluginRequest>>>,
    /// 下行事件广播。
    events: broadcast::Sender<ServerMessage>,
}

impl AppState {
    /// 构造一份共享状态。
    ///
    /// 构造时会读取设置，并把「第一个有 API key 的服务」设为当前生效服务；
    /// 若没有任何可用服务，则当前生效服务为 `None`。
    pub async fn new(store: SettingsStore) -> Self {
        let active = {
            let settings = store.load().await.unwrap_or_default();
            settings
                .services
                .iter()
                .find(|(_, service)| service.has_api_key())
                .map(|(id, _)| id.clone())
        };

        let (events, _) = broadcast::channel(BROADCAST_CAPACITY);

        Self(Arc::new(Inner {
            store,
            active_service: RwLock::new(active),
            active_turn: RwLock::new(None),
            plugins: RwLock::new(HashMap::new()),
            events,
        }))
    }

    /// 返回设置存储的句柄。
    pub fn store(&self) -> &SettingsStore {
        &self.0.store
    }

    /// 订阅下行事件流。
    pub fn subscribe(&self) -> broadcast::Receiver<ServerMessage> {
        self.0.events.subscribe()
    }

    /// 当前生效的服务标识。
    pub fn active_service(&self) -> Option<String> {
        self.0
            .active_service
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// 清空当前生效的服务标识（例如生效服务被删除且没有其它可用服务时）。
    pub fn clear_active_service(&self) {
        *self
            .0
            .active_service
            .write()
            .unwrap_or_else(|err| err.into_inner()) = None;
    }

    /// 切换当前生效的服务；服务不存在时返回错误。
    pub async fn use_service(&self, service_id: &str) -> Result<(), HubError> {
        let settings =
            self.0.store.load().await.map_err(|err| {
                HubError::new(ErrorCode::Internal, format!("读取设置失败: {err}"))
            })?;

        if !settings.services.contains_key(service_id) {
            return Err(HubError::new(
                ErrorCode::UnknownService,
                format!("服务 {service_id} 不存在"),
            ));
        }

        *self
            .0
            .active_service
            .write()
            .unwrap_or_else(|err| err.into_inner()) = Some(service_id.to_string());

        Ok(())
    }

    /// 当前是否有插件在线。
    pub fn plugin_online(&self) -> bool {
        !self
            .0
            .plugins
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .is_empty()
    }

    /// 当前可用的服务标识列表。
    pub async fn service_ids(&self) -> Vec<String> {
        self.0
            .store
            .load()
            .await
            .map(|settings| settings.services.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// 生成 `ready` 帧。
    pub async fn ready_message(&self) -> ServerMessage {
        ServerMessage::Ready {
            plugin_online: self.plugin_online(),
            services: self.service_ids().await,
            active_service: self.active_service(),
            server_version: SERVER_VERSION.to_string(),
        }
    }

    /// 向所有订阅者广播一条消息。
    ///
    /// 没有订阅者时发送会失败，这里视为正常情况（还没有页面打开）。
    pub fn broadcast(&self, message: ServerMessage) {
        let _ = self.0.events.send(message);
    }

    /// 注册一个插件连接，返回连接标识与待写出的指令队列。
    ///
    /// 调用方应当在连接结束时调用 [`AppState::unregister_plugin`]。
    pub fn register_plugin(&self, id: Uuid) -> mpsc::Receiver<PluginRequest> {
        let (tx, rx) = mpsc::channel(PLUGIN_CHANNEL_CAPACITY);

        self.0
            .plugins
            .write()
            .unwrap_or_else(|err| err.into_inner())
            .insert(id, tx);

        rx
    }

    /// 注销一个插件连接。
    ///
    /// 若注销时仍有进行中的 turn，会广播一条「插件离线」错误并清空 turn 状态，
    /// 避免浏览器永远停在生成中。
    pub fn unregister_plugin(&self, id: Uuid) {
        let removed = self
            .0
            .plugins
            .write()
            .unwrap_or_else(|err| err.into_inner())
            .remove(&id);

        if removed.is_none() {
            return;
        }

        let turn_id = self.take_active_turn().map(|turn| turn.turn_id);

        if let Some(turn_id) = turn_id {
            self.broadcast(ServerMessage::Error {
                turn_id: Some(turn_id),
                code: ErrorCode::PluginOffline,
                message: "插件在生成过程中断开连接".to_string(),
            });
        }
    }

    /// 处理一条来自浏览器的消息。
    ///
    /// 返回给调用方的是「需要直接回给该连接」的错误；正常事件一律走广播。
    pub async fn handle_client_message(&self, message: ClientMessage) -> Result<(), HubError> {
        match message {
            ClientMessage::Ask {
                turn_id,
                question,
                service_id,
            } => self.handle_ask(turn_id, question, service_id).await,
            ClientMessage::Cancel { turn_id } => self.handle_cancel(&turn_id),
            ClientMessage::UseService { service_id } => self.use_service(&service_id).await,
        }
    }

    /// 处理一次提问。
    async fn handle_ask(
        &self,
        turn_id: Option<String>,
        question: String,
        service_id: Option<String>,
    ) -> Result<(), HubError> {
        let turn_id = turn_id.unwrap_or_else(|| Uuid::new_v4().simple().to_string());

        if question.trim().is_empty() {
            return Err(HubError::for_turn(
                turn_id,
                ErrorCode::BadRequest,
                "问题不能为空",
            ));
        }

        if self
            .0
            .active_turn
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .is_some()
        {
            return Err(HubError::for_turn(
                turn_id,
                ErrorCode::BadRequest,
                "上一轮仍在生成中，请先取消",
            ));
        }

        let settings = self.0.store.load().await.map_err(|err| {
            HubError::for_turn(
                turn_id.clone(),
                ErrorCode::Internal,
                format!("读取设置失败: {err}"),
            )
        })?;

        let service_id = match service_id.or_else(|| self.active_service()) {
            Some(id) => id,
            // 没有显式指定、也没有生效服务时，退而选第一个「已配置 API key」的服务。
            // 不能直接取第一个服务：它可能还没有 key，那样错误信息会误导用户。
            None => match settings
                .services
                .iter()
                .find(|(_, service)| service.has_api_key())
                .map(|(id, _)| id.clone())
            {
                Some(id) => id,
                None => {
                    return Err(HubError::for_turn(
                        turn_id.clone(),
                        ErrorCode::UnknownService,
                        "尚未配置任何可用的 LLM 服务",
                    ));
                }
            },
        };

        let service: LlmServiceConfig = match settings.services.get(&service_id) {
            Some(service) => service.clone(),
            None => {
                return Err(HubError::for_turn(
                    turn_id.clone(),
                    ErrorCode::UnknownService,
                    format!("服务 {service_id} 不存在"),
                ));
            }
        };

        if !service.has_api_key() {
            return Err(HubError::for_turn(
                turn_id.clone(),
                ErrorCode::MissingApiKey,
                format!("服务 {service_id} 还未配置 API key，请在设置里填写"),
            ));
        }

        let request = PluginRequest::Ask {
            turn_id: turn_id.clone(),
            service_id: service_id.clone(),
            service,
            question,
        };

        if !self.send_to_plugin(request) {
            return Err(HubError::for_turn(
                turn_id,
                ErrorCode::PluginOffline,
                "LLM 插件当前未连接",
            ));
        }

        *self
            .0
            .active_turn
            .write()
            .unwrap_or_else(|err| err.into_inner()) = Some(ActiveTurn {
            turn_id,
            service_id,
            started: false,
        });

        Ok(())
    }

    /// 处理一次取消。
    fn handle_cancel(&self, turn_id: &str) -> Result<(), HubError> {
        let active = self
            .0
            .active_turn
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone();

        let Some(active) = active.filter(|turn| turn.turn_id == turn_id) else {
            return Err(HubError::for_turn(
                turn_id,
                ErrorCode::BadRequest,
                "没有正在进行的该轮生成",
            ));
        };

        let _ = self.send_to_plugin(PluginRequest::Cancel {
            turn_id: active.turn_id.clone(),
        });

        // 立刻把取消结果告诉界面：不等待插件回执，避免取消按钮看起来「没反应」。
        self.broadcast(ServerMessage::Finished {
            turn_id: active.turn_id.clone(),
            reason: Some(FinishReason::Cancelled),
        });
        *self
            .0
            .active_turn
            .write()
            .unwrap_or_else(|err| err.into_inner()) = None;

        Ok(())
    }

    /// 把一条指令交给任意一个在线插件；没有插件在线时返回 `false`。
    fn send_to_plugin(&self, request: PluginRequest) -> bool {
        let senders: Vec<mpsc::Sender<PluginRequest>> = self
            .0
            .plugins
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .values()
            .cloned()
            .collect();

        for sender in senders {
            match sender.try_send(request.clone()) {
                Ok(()) => return true,
                // 队列满说明该插件卡住，换下一个；Closed 说明连接已失效。
                Err(mpsc::error::TrySendError::Full(_)) => continue,
                Err(mpsc::error::TrySendError::Closed(_)) => continue,
            }
        }

        false
    }

    /// 处理一条来自插件的事件。
    ///
    /// # 两条路径
    ///
    /// - **信封路径**：`Started` / `Finished` / `Error` 由服务端直接处理，用来维护
    ///   turn 状态；
    /// - **内容路径**：`Raw` 里装的是 rig 的原始 JSON，服务端**不理解它**，只是转交给
    ///   [`kb_rig_llm_v1_adapt`] 翻译成 `abs_llm::v1` 形状，再广播给浏览器
    ///   （`dev-notes.md` §2.2 / §2.3 / §2.4）。
    pub fn handle_plugin_event(&self, event: PluginEvent) {
        match event {
            PluginEvent::Hello { .. } | PluginEvent::Pong => {
                // 握手与心跳不产生用户可见输出。
            }
            PluginEvent::Started {
                turn_id,
                model,
                capabilities,
            } => {
                let mut guard = self
                    .0
                    .active_turn
                    .write()
                    .unwrap_or_else(|err| err.into_inner());

                let Some(turn) = guard.as_mut().filter(|turn| turn.turn_id == turn_id) else {
                    log::debug!("忽略与当前 turn 无关的 started: {turn_id}");
                    return;
                };

                turn.started = true;
                let service_id = turn.service_id.clone();
                drop(guard);

                self.broadcast(ServerMessage::Started {
                    turn_id,
                    service_id,
                    model,
                    capabilities,
                });
            }
            PluginEvent::Raw { turn_id, payload } => {
                self.handle_raw_payload(&turn_id, &payload);
            }
            PluginEvent::Finished { turn_id, reason } => {
                if !self.is_active_turn(&turn_id) {
                    return;
                }

                self.broadcast(ServerMessage::Finished {
                    turn_id: turn_id.clone(),
                    reason,
                });
                let _ = self.take_active_turn();
            }
            PluginEvent::Error { turn_id, message } => {
                if let Some(turn_id) = &turn_id
                    && !self.is_active_turn(turn_id)
                {
                    return;
                }

                self.broadcast(ServerMessage::Error {
                    turn_id: turn_id.clone(),
                    code: ErrorCode::Provider,
                    message,
                });
                let _ = self.take_active_turn();
            }
        }
    }

    /// 把一条 rig 原始载荷翻译成 `abs_llm::v1` 形状并广播。
    ///
    /// 翻译失败的载荷**只记日志不报错**：provider 会不断新增分片类型，
    /// 为了一个不认识的字段就把整轮对话打断是不划算的。
    fn handle_raw_payload(&self, turn_id: &str, payload: &serde_json::Value) {
        if !self.is_active_turn(turn_id) {
            log::debug!("忽略与当前 turn 无关的原始载荷: {turn_id}");
            return;
        }

        // 1) 先看它是不是「用量」——rig 把用量放在最终响应里，而不是普通分片。
        if let Some(usage) = AdaptedEvent::usage_from_payload(payload) {
            self.broadcast(ServerMessage::Usage {
                turn_id: turn_id.to_string(),
                usage: Usage {
                    input_tokens: usage.input().map(|value| value as u64),
                    output_tokens: usage.output().map(|value| value as u64),
                    total_tokens: usage.total().map(|value| value as u64),
                },
            });
            return;
        }

        // 2) 再看它是不是结束原因。
        if let Some(reason) = payload
            .get("finish_reason")
            .and_then(|value| value.as_str())
            .and_then(AdaptedEvent::finish_reason_from_str)
        {
            self.broadcast(ServerMessage::Finished {
                turn_id: turn_id.to_string(),
                reason: Some(to_wire_finish_reason(reason)),
            });
            let _ = self.take_active_turn();
            return;
        }

        // 3) 其余按内容分片处理。
        let Some(event) = AdaptedEvent::from_payload(payload) else {
            log::debug!("无法转换的插件载荷（已忽略）: {payload}");
            return;
        };

        match event {
            AdaptedEvent::TextDelta(delta) => {
                self.broadcast(ServerMessage::Delta {
                    turn_id: turn_id.to_string(),
                    logic: to_wire_logic(delta.logic_kind()),
                    text: delta.text_ref().to_string(),
                });
            }
            AdaptedEvent::ToolCall(call) => {
                self.broadcast(ServerMessage::ToolCall {
                    turn_id: turn_id.to_string(),
                    id: call.id_ref().to_string(),
                    name: call.name_ref().to_string(),
                    arguments: call.arguments_ref().to_string(),
                });
            }
            AdaptedEvent::Usage(usage) => {
                self.broadcast(ServerMessage::Usage {
                    turn_id: turn_id.to_string(),
                    usage: Usage {
                        input_tokens: usage.input().map(|value| value as u64),
                        output_tokens: usage.output().map(|value| value as u64),
                        total_tokens: usage.total().map(|value| value as u64),
                    },
                });
            }
            AdaptedEvent::Finished { reason } => {
                self.broadcast(ServerMessage::Finished {
                    turn_id: turn_id.to_string(),
                    reason: reason.map(to_wire_finish_reason),
                });
                let _ = self.take_active_turn();
            }
        }
    }

    /// 判断某个 turn 是否为当前进行中的 turn。
    fn is_active_turn(&self, turn_id: &str) -> bool {
        self.0
            .active_turn
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .as_ref()
            .is_some_and(|turn| turn.turn_id == turn_id)
    }

    /// 取出并清空当前 turn。
    fn take_active_turn(&self) -> Option<ActiveTurn> {
        self.0
            .active_turn
            .write()
            .unwrap_or_else(|err| err.into_inner())
            .take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{Capabilities, LogicOutput};

    /// 构造一份带 `deepseek` 服务的共享状态（内存存储，不触碰文件系统）。
    async fn state_with_service(with_key: bool) -> AppState {
        let store = SettingsStore::memory();
        store
            .upsert_service(
                "deepseek",
                &LlmServiceConfig::new(
                    "deepseek",
                    "deepseek-chat",
                    "",
                    if with_key { "sk-test" } else { "" },
                ),
            )
            .await
            .expect("写入内存设置应当成功");

        AppState::new(store).await
    }

    /// 测试没有任何服务时提问会被拒绝，并给出 `unknown_service` 错误。
    ///
    /// - 手段：用空的内存存储构造状态，调用 `handle_client_message` 发出一问。
    /// - 判断：返回 `Err(HubError { code: UnknownService, .. })`。
    #[tokio::test]
    async fn ask_without_service_is_rejected() {
        let state = AppState::new(SettingsStore::memory()).await;

        let result = state
            .handle_client_message(ClientMessage::Ask {
                turn_id: Some("t1".to_string()),
                question: "你好".to_string(),
                service_id: None,
            })
            .await;

        match result {
            Err(HubError {
                code: ErrorCode::UnknownService,
                ..
            }) => {}
            other => panic!("应当因缺少服务被拒绝，实际: {other:?}"),
        }
    }

    /// 测试「服务存在但没有 API key」时给出 `missing_api_key`。
    ///
    /// - 手段：写入一个 `api_key` 为空的服务，并**显式指定**该服务提问。
    /// - 判断：错误类别为 `MissingApiKey`，说明用户被明确告知「缺 key」，
    ///   而不是笼统的「没有可用服务」。
    #[tokio::test]
    async fn ask_without_api_key_is_rejected() {
        let state = state_with_service(false).await;

        let result = state
            .handle_client_message(ClientMessage::Ask {
                turn_id: Some("t1".to_string()),
                question: "你好".to_string(),
                service_id: Some("deepseek".to_string()),
            })
            .await;

        match result {
            Err(HubError {
                code: ErrorCode::MissingApiKey,
                ..
            }) => {}
            other => panic!("应当因缺少 API key 被拒绝，实际: {other:?}"),
        }
    }

    /// 测试「只有一个服务且它没有 key、又未指定服务」时给出 `unknown_service`。
    ///
    /// - 手段：写入一个 `api_key` 为空的服务，不指定 `service_id` 提问。
    /// - 判断：错误类别为 `UnknownService`——此时**不能**把没有 key 的服务
    ///   当成可用的兜底选项，否则用户会遇到含糊的失败。
    #[tokio::test]
    async fn ask_falls_back_only_to_services_with_key() {
        let state = state_with_service(false).await;

        let result = state
            .handle_client_message(ClientMessage::Ask {
                turn_id: Some("t1".to_string()),
                question: "你好".to_string(),
                service_id: None,
            })
            .await;

        match result {
            Err(HubError {
                code: ErrorCode::UnknownService,
                ..
            }) => {}
            other => panic!("应当报告没有可用服务，实际: {other:?}"),
        }
    }

    /// 测试没有插件在线时提问返回 `plugin_offline`。
    ///
    /// - 手段：写入一个带 key 的服务后提问，此时没有任何插件注册。
    /// - 判断：错误类别为 `PluginOffline`。
    #[tokio::test]
    async fn ask_without_plugin_reports_offline() {
        let state = state_with_service(true).await;

        let result = state
            .handle_client_message(ClientMessage::Ask {
                turn_id: Some("t1".to_string()),
                question: "你好".to_string(),
                service_id: None,
            })
            .await;

        match result {
            Err(HubError {
                code: ErrorCode::PluginOffline,
                ..
            }) => {}
            other => panic!("应当报告插件离线，实际: {other:?}"),
        }
    }

    /// 测试完整的转发链路：提问 → 插件收到指令 → 插件事件广播给浏览器。
    ///
    /// - 手段：注册一个插件连接，提问后从插件的接收队列取出 `ask` 指令；
    ///   随后依次注入 `started`、两条 `delta`、`usage`、`finished` 事件，
    ///   并从广播订阅者处按序读取。
    /// - 判断：插件收到的问题与选项正确；广播序列为
    ///   `started → delta(answer) → delta(reasoning) → usage → finished`，
    ///   且所有帧的 `turn_id` 一致。
    #[tokio::test]
    async fn ask_is_forwarded_and_events_are_broadcast() {
        let state = state_with_service(true).await;
        let mut events = state.subscribe();
        let plugin_id = Uuid::new_v4();
        let mut plugin_rx = state.register_plugin(plugin_id);

        state
            .handle_client_message(ClientMessage::Ask {
                turn_id: Some("t1".to_string()),
                question: "介绍一下你自己".to_string(),
                service_id: None,
            })
            .await
            .expect("应当成功转发给插件");

        match plugin_rx.recv().await.expect("插件应当收到指令") {
            PluginRequest::Ask {
                turn_id,
                service_id,
                service,
                question,
            } => {
                assert_eq!(turn_id, "t1");
                assert_eq!(service_id, "deepseek");
                assert_eq!(service.model, "deepseek-chat");
                assert_eq!(question, "介绍一下你自己");
            }
            other => panic!("收到的应当是一条 ask，实际: {other:?}"),
        }

        state.handle_plugin_event(PluginEvent::Started {
            turn_id: "t1".to_string(),
            model: "deepseek-chat".to_string(),
            capabilities: Capabilities::default(),
        });
        // 以下三条都是「rig 的原始载荷」，由 adapter 翻译成 abs_llm::v1 形状。
        state.handle_plugin_event(PluginEvent::Raw {
            turn_id: "t1".to_string(),
            payload: serde_json::json!({ "type": "text_delta", "text": "你好" }),
        });
        state.handle_plugin_event(PluginEvent::Raw {
            turn_id: "t1".to_string(),
            payload: serde_json::json!({ "type": "reasoning_delta", "reasoning": "先想想" }),
        });
        state.handle_plugin_event(PluginEvent::Raw {
            turn_id: "t1".to_string(),
            payload: serde_json::json!({ "prompt_tokens": 10, "completion_tokens": 2 }),
        });
        state.handle_plugin_event(PluginEvent::Finished {
            turn_id: "t1".to_string(),
            reason: Some(FinishReason::Completed),
        });

        let mut seen = Vec::new();
        for _ in 0..5 {
            seen.push(events.recv().await.expect("应当能读到广播事件"));
        }

        assert!(matches!(
            &seen[0],
            ServerMessage::Started { turn_id, model, .. } if turn_id == "t1" && model == "deepseek-chat"
        ));
        assert!(matches!(
            &seen[1],
            ServerMessage::Delta { logic: LogicOutput::Answer, text, .. } if text == "你好"
        ));
        assert!(matches!(
            &seen[2],
            ServerMessage::Delta { logic: LogicOutput::Reasoning, text, .. } if text == "先想想"
        ));
        assert!(matches!(
            &seen[3],
            ServerMessage::Usage { usage, .. } if usage.total_tokens == Some(12)
        ));
        assert!(matches!(
            &seen[4],
            ServerMessage::Finished {
                reason: Some(FinishReason::Completed),
                ..
            }
        ));

        // turn 结束后应当回到空闲，允许下一次提问。
        assert!(state.take_active_turn().is_none());
    }

    /// 测试取消会立刻广播 `finished(cancelled)` 并清空 turn。
    ///
    /// - 手段：先提问建立 turn，再发送 `cancel`。
    /// - 判断：插件收到 `cancel` 指令；广播中含 `Finished { reason: Cancelled }`。
    #[tokio::test]
    async fn cancel_finishes_turn_immediately() {
        let state = state_with_service(true).await;
        let mut events = state.subscribe();
        let plugin_id = Uuid::new_v4();
        let mut plugin_rx = state.register_plugin(plugin_id);

        state
            .handle_client_message(ClientMessage::Ask {
                turn_id: Some("t1".to_string()),
                question: "写一首诗".to_string(),
                service_id: None,
            })
            .await
            .expect("提问应当成功");
        let _ = plugin_rx.recv().await.expect("应当先收到 ask");

        state
            .handle_client_message(ClientMessage::Cancel {
                turn_id: "t1".to_string(),
            })
            .await
            .expect("取消应当成功");

        match plugin_rx.recv().await.expect("插件应当收到取消指令") {
            PluginRequest::Cancel { turn_id } => assert_eq!(turn_id, "t1"),
            other => panic!("应当收到 cancel，实际: {other:?}"),
        }

        match events.recv().await.expect("应当有广播") {
            ServerMessage::Finished { turn_id, reason } => {
                assert_eq!(turn_id, "t1");
                assert_eq!(reason, Some(FinishReason::Cancelled));
            }
            other => panic!("应当收到 finished，实际: {other:?}"),
        }
    }

    /// 测试插件断线会结束进行中的 turn 并广播错误。
    ///
    /// - 手段：提问后注销插件连接。
    /// - 判断：广播中出现 `Error { code: PluginOffline }`，且随后可以再次提问。
    #[tokio::test]
    async fn plugin_disconnect_fails_active_turn() {
        let state = state_with_service(true).await;
        let mut events = state.subscribe();
        let plugin_id = Uuid::new_v4();
        let _plugin_rx = state.register_plugin(plugin_id);

        state
            .handle_client_message(ClientMessage::Ask {
                turn_id: Some("t1".to_string()),
                question: "在吗".to_string(),
                service_id: None,
            })
            .await
            .expect("提问应当成功");

        state.unregister_plugin(plugin_id);

        match events.recv().await.expect("应当有广播") {
            ServerMessage::Error {
                turn_id: Some(turn_id),
                code: ErrorCode::PluginOffline,
                ..
            } => assert_eq!(turn_id, "t1"),
            other => panic!("应当收到插件离线错误，实际: {other:?}"),
        }

        assert!(state.take_active_turn().is_none(), "turn 状态应当被清空");
    }

    /// 测试无关 turn 的事件会被忽略。
    ///
    /// - 手段：在没有进行中 turn 的情况下注入一条 `delta`。
    /// - 判断：广播订阅者读不到任何消息。
    #[tokio::test]
    async fn unrelated_events_are_ignored() {
        let state = state_with_service(true).await;
        let mut events = state.subscribe();

        state.handle_plugin_event(PluginEvent::Raw {
            turn_id: "ghost".to_string(),
            payload: serde_json::json!({ "type": "text_delta", "text": "不该出现" }),
        });

        assert!(events.try_recv().is_err(), "无关事件不应产生广播");
    }

    /// 测试 `ready` 帧如实反映插件与服务状态。
    ///
    /// - 手段：构造一个没有插件的状态，读取 `ready_message`。
    /// - 判断：`plugin_online` 为 `false`；`active_service` 为 `Some("deepseek")`
    ///   （因为该服务配置了 key）；`services` 含 `deepseek`。
    #[tokio::test]
    async fn ready_reports_current_state() {
        let state = state_with_service(true).await;

        match state.ready_message().await {
            ServerMessage::Ready {
                plugin_online,
                services,
                active_service,
                server_version,
            } => {
                assert!(!plugin_online);
                assert_eq!(services, vec!["deepseek".to_string()]);
                assert_eq!(active_service.as_deref(), Some("deepseek"));
                assert!(!server_version.is_empty());
            }
            other => panic!("应当返回 ready，实际: {other:?}"),
        }
    }
}
