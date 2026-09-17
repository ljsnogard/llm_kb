//! 请求 / 应答的关联信封。
//!
//! 一个客户端与 `kb_core` 之间可以只开**一对**通道（一个上行、一个下行），
//! 由 [`RequestEnvelope::request_id`] 把应答配回请求；也可以每个请求开一对通道，
//! 此时 `request_id` 依然有用（用于日志与排错）。
//!
//! 事件不需要信封——它是单向推送，没有"配回哪个请求"的问题。

use serde::{Deserialize, Serialize};

use super::ids_::RequestId;
use super::reply_::Reply;
use super::request_::Request;

/// 请求信封：把请求与它的关联标识绑在一起。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    /// 请求标识，由客户端生成，在本次连接内唯一。
    pub request_id: RequestId,

    /// 请求内容。
    pub request: Request,
}

impl RequestEnvelope {
    /// 构造一个请求信封。
    pub fn new(request_id: impl Into<RequestId>, request: Request) -> Self {
        Self {
            request_id: request_id.into(),
            request,
        }
    }
}

/// 应答信封。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplyEnvelope {
    /// 对应请求的标识。
    pub request_id: RequestId,

    /// 应答内容。
    pub reply: Reply,
}

impl ReplyEnvelope {
    /// 构造一个应答信封。
    pub fn new(request_id: impl Into<RequestId>, reply: Reply) -> Self {
        Self {
            request_id: request_id.into(),
            reply,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1::desktop::{
        AskRequest, ClientInfo, DirectoryListing, Event, LocalId, PROTOCOL_VERSION, ServerInfo,
        ServerState, ServiceId, ServiceList, SessionId, SessionSummary, TextDelta, TurnId,
        TurnState, Workspace, WorkspaceId, WorkspaceList,
    };
    use abs_llm::v1::cont::LogicOutput;

    /// 取一组覆盖各方向的代表性消息。
    ///
    /// 这些用例刻意覆盖三类结构：单元变体、newtype 变体、结构体变体，
    /// 以及"含嵌套结构体/枚举"的载荷。
    fn sample_messages() -> (Vec<RequestEnvelope>, Vec<ReplyEnvelope>, Vec<Event>) {
        let workspace_id = WorkspaceId::new("w-1");
        let session_id = SessionId::new("s-1");
        let turn_id = TurnId::new("t-1");

        let requests = vec![
            RequestEnvelope::new(
                "q-1",
                Request::Hello(ClientInfo {
                    client_name: "kb_admin_desktop".to_string(),
                    client_version: "0.1.0".to_string(),
                    protocol_version: PROTOCOL_VERSION,
                }),
            ),
            RequestEnvelope::new(
                "q-2",
                Request::Ask(AskRequest {
                    workspace_id: workspace_id.clone(),
                    session_id: session_id.clone(),
                    turn_id: turn_id.clone(),
                    question: "你好".to_string(),
                    service_id: Some(ServiceId::new("deepseek")),
                }),
            ),
            RequestEnvelope::new(
                "q-3",
                Request::Cancel {
                    turn_id: turn_id.clone(),
                },
            ),
            RequestEnvelope::new("q-4", Request::ListServices),
            RequestEnvelope::new(
                "q-5",
                Request::AddWorkspace(crate::v1::desktop::AddWorkspaceRequest {
                    local_id: LocalId::new("l-1"),
                    name: "笔记".to_string(),
                    path: "/home/me/notes".to_string(),
                }),
            ),
            RequestEnvelope::new(
                "q-6",
                Request::ListDirectory {
                    workspace_id: workspace_id.clone(),
                    relative_path: "notes".to_string(),
                },
            ),
        ];

        let replies = vec![
            ReplyEnvelope::new(
                "q-1",
                Reply::Hello(ServerInfo {
                    server_version: "0.1.0".to_string(),
                    protocol_version: PROTOCOL_VERSION,
                }),
            ),
            ReplyEnvelope::new("q-2", Reply::Ack),
            ReplyEnvelope::new(
                "q-3",
                Reply::WorkspaceAdded {
                    local_id: LocalId::new("l-1"),
                    workspace: Workspace {
                        workspace_id: workspace_id.clone(),
                        name: "笔记".to_string(),
                        path: "/home/me/notes".to_string(),
                    },
                },
            ),
            ReplyEnvelope::new(
                "q-4",
                Reply::ServiceList(ServiceList::default()),
            ),
            ReplyEnvelope::new(
                "q-5",
                Reply::SessionCreated {
                    local_id: LocalId::new("l-2"),
                    session: SessionSummary {
                        session_id: session_id.clone(),
                        workspace_id: workspace_id.clone(),
                        title: "新会话".to_string(),
                        updated_at_millis: 1_760_000_000_000,
                        turn_count: 0,
                    },
                },
            ),
            ReplyEnvelope::new(
                "q-6",
                Reply::DirectoryListing(DirectoryListing {
                    workspace_id: workspace_id.clone(),
                    relative_path: "notes".to_string(),
                    entries: Vec::new(),
                }),
            ),
            ReplyEnvelope::new(
                "q-7",
                Reply::WorkspaceList(WorkspaceList {
                    workspaces: vec![Workspace {
                        workspace_id: workspace_id.clone(),
                        name: "笔记".to_string(),
                        path: "/home/me/notes".to_string(),
                    }],
                }),
            ),
        ];

        let events = vec![
            Event::Ready(ServerState {
                plugin_online: true,
                server_version: "0.1.0".to_string(),
                services: Vec::new(),
                active_service: Some(ServiceId::new("deepseek")),
            }),
            Event::Delta(TextDelta {
                turn_id: turn_id.clone(),
                logic: LogicOutput::Answer,
                text: "你好".to_string(),
            }),
            Event::TurnFinished(crate::v1::desktop::TurnFinished {
                turn_id: turn_id.clone(),
                reason: None,
            }),
            Event::SessionChanged(crate::v1::desktop::SessionChanged {
                summary: SessionSummary {
                    session_id: session_id.clone(),
                    workspace_id: workspace_id.clone(),
                    title: "新会话".to_string(),
                    updated_at_millis: 1,
                    turn_count: 1,
                },
            }),
            Event::StateChanged(ServerState::default()),
        ];

        let _ = TurnState::Done;
        (requests, replies, events)
    }

    /// 测试所有代表性消息都能通过 **postcard** 往返。
    ///
    /// - 手段：取覆盖三个方向的代表性消息，用 `postcard::to_allocvec` 编码后
    ///   `postcard::from_bytes` 解码。
    /// - 判断：每一条都解码成功且与原值相等。
    ///
    /// 这条测试守住的是 [`desktop`](crate::v1::desktop) 模块文档里那条约定：
    /// **不能使用 serde 的内部标签枚举**——postcard 不自描述，内部标签的
    /// 解码会以 "This is a feature that PostCard will never implement" 失败。
    /// 该失败只在解码侧出现（编码会"成功"地产生垃圾），因此必须用真解码来守。
    #[test]
    fn every_sample_message_round_trips_through_postcard_() {
        let (requests, replies, events) = sample_messages();

        for envelope in &requests {
            let bytes = postcard::to_allocvec(envelope).expect("postcard 应当能编码请求");
            let parsed: RequestEnvelope =
                postcard::from_bytes(&bytes).expect("postcard 应当能解码请求");
            assert_eq!(&parsed, envelope);
        }

        for envelope in &replies {
            let bytes = postcard::to_allocvec(envelope).expect("postcard 应当能编码应答");
            let parsed: ReplyEnvelope =
                postcard::from_bytes(&bytes).expect("postcard 应当能解码应答");
            assert_eq!(&parsed, envelope);
        }

        for event in &events {
            let bytes = postcard::to_allocvec(event).expect("postcard 应当能编码事件");
            let parsed: Event = postcard::from_bytes(&bytes).expect("postcard 应当能解码事件");
            assert_eq!(&parsed, event);
        }
    }

    /// 测试同一批消息在 JSON 上也能往返（便于排查问题）。
    ///
    /// - 手段：对同一批代表性消息做 `serde_json` 往返。
    /// - 判断：全部与原值相等——即协议不依赖 postcard 的私有特性，
    ///   两种表示都能用。
    #[test]
    fn every_sample_message_round_trips_through_json_() {
        let (requests, replies, events) = sample_messages();

        for envelope in &requests {
            let json = serde_json::to_string(envelope).expect("应当能序列化");
            let parsed: RequestEnvelope = serde_json::from_str(&json).expect("应当能反序列化");
            assert_eq!(&parsed, envelope);
        }

        for envelope in &replies {
            let json = serde_json::to_string(envelope).expect("应当能序列化");
            let parsed: ReplyEnvelope = serde_json::from_str(&json).expect("应当能反序列化");
            assert_eq!(&parsed, envelope);
        }

        for event in &events {
            let json = serde_json::to_string(event).expect("应当能序列化");
            let parsed: Event = serde_json::from_str(&json).expect("应当能反序列化");
            assert_eq!(&parsed, event);
        }
    }

    /// 测试信封把请求与关联标识绑在一起。
    ///
    /// - 手段：构造 `RequestEnvelope` 并序列化。
    /// - 判断：JSON 同时含 `request_id` 与 `request`；往返后标识保持不变。
    #[test]
    fn envelope_keeps_request_id_() {
        let envelope = RequestEnvelope::new("q-9", Request::ListServices);
        let json = serde_json::to_string(&envelope).expect("应当能序列化");
        assert!(json.contains(r#""request_id":"q-9""#), "实际 JSON: {json}");
        assert!(json.contains(r#""request":"#), "实际 JSON: {json}");

        let parsed: RequestEnvelope = serde_json::from_str(&json).expect("应当能反序列化");
        assert_eq!(parsed.request_id, RequestId::new("q-9"));
    }
}
