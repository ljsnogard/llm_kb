//! 客户端 → `kb_core` 的请求。
//!
//! # 表示形式
//!
//! 一律使用 serde 的**外部标签**（serde 默认），即 `{"Ask": {…}}`。
//! 不能用 `#[serde(tag = "type")]`——目标传输的编解码器（postcard）不支持，
//! 详见 [`desktop`](crate::v1::desktop) 模块文档。

use serde::{Deserialize, Serialize};

use super::content_::Turn;
use super::handshake_::ClientInfo;
use super::ids_::{LocalId, ServiceId, SessionId, TurnId, WorkspaceId};
use super::service_::ApiKeyUpdate;

/// 一次提问。
///
/// 回合标识 [`AskRequest::turn_id`] 由**客户端**生成：这样从"发出提问"到
/// "收到第一条增量"之间的窗口期里，界面也已经有一个标识可用。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AskRequest {
    /// 提问所在的工作区。
    pub workspace_id: WorkspaceId,

    /// 提问所在的会话。
    pub session_id: SessionId,

    /// 客户端生成的回合标识；服务端以此回报增量。
    pub turn_id: TurnId,

    /// 问题正文。
    pub question: String,

    /// 指定使用的服务；`None` 表示用当前生效的服务。
    #[serde(default)]
    pub service_id: Option<ServiceId>,
}

/// 新增或覆盖一个 LLM 服务。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertServiceRequest {
    /// 服务标识（用户可见的名字）。
    pub service_id: ServiceId,

    /// provider 标识。
    pub provider: String,

    /// 模型名。
    pub model: String,

    /// API base URL；空表示用 provider 默认地址。
    #[serde(default)]
    pub base_url: String,

    /// API key 的更新语义。
    pub api_key: ApiKeyUpdate,
}

/// 同步一个客户端**本地创建**的工作区。
///
/// 客户端可以在没有连上 `kb_core` 时就创建好工作区（目录是本地路径），
/// 同步时把它连同 [`LocalId`] 一起提交；`kb_core` 负责分配真正的
/// [`WorkspaceId`](super::ids_::WorkspaceId) 并在应答里回传配对关系。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddWorkspaceRequest {
    /// 客户端创建该工作区时使用的临时标识。
    ///
    /// 同步完成后客户端应当用应答里的 `workspace_id` 取代它。
    pub local_id: LocalId,

    /// 展示名。
    pub name: String,

    /// 磁盘目录。
    pub path: String,
}

/// 在某个工作区里同步一个客户端**本地创建**的会话。
///
/// 必须先同步工作区：`workspace_id` 只能来自
/// [`AddWorkspaceRequest`] 的应答。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    /// 目标工作区（`kb_core` 分配的标识）。
    pub workspace_id: WorkspaceId,

    /// 客户端创建该会话时使用的临时标识。
    pub local_id: LocalId,

    /// 标题；`None` 表示由服务端取默认值。
    #[serde(default)]
    pub title: Option<String>,

    /// 客户端在同步之前**已经攒下的消息**，按时间顺序。
    ///
    /// 通常为空；离线使用过的客户端会带上它，这样历史不会丢。
    #[serde(default)]
    pub turns: Vec<Turn>,
}

/// 桌面客户端发给 `kb_core` 的请求。
///
/// # 示例
///
/// ```
/// use abs_kb_svc::v1::desktop::{Request, ServiceId};
///
/// let request = Request::UseService {
///     service_id: ServiceId::new("deepseek"),
/// };
///
/// let json = serde_json::to_string(&request).expect("应当能序列化");
/// // 外部标签：变体名就是 JSON 的键
/// assert!(json.starts_with(r#"{"UseService""#), "实际 JSON: {json}");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    /// 握手，必须是最先发出的一条。
    Hello(ClientInfo),

    /// 就某个会话提问。
    Ask(AskRequest),

    /// 取消某一轮生成。
    Cancel {
        /// 要取消的回合。
        turn_id: TurnId,
    },

    /// 列出已配置的 LLM 服务。
    ListServices,

    /// 新增或覆盖一个 LLM 服务。
    UpsertService(UpsertServiceRequest),

    /// 删除一个 LLM 服务。
    RemoveService {
        /// 要删除的服务。
        service_id: ServiceId,
    },

    /// 切换当前生效的 LLM 服务。
    UseService {
        /// 目标服务。
        service_id: ServiceId,
    },

    /// 列出全部工作区。
    ListWorkspaces,

    /// 同步一个本地创建的工作区，由 `kb_core` 分配标识。
    AddWorkspace(AddWorkspaceRequest),

    /// 删除一个工作区。
    RemoveWorkspace {
        /// 要删除的工作区。
        workspace_id: WorkspaceId,
    },

    /// 列出某个工作区下的会话。
    ListSessions {
        /// 目标工作区。
        workspace_id: WorkspaceId,
    },

    /// 同步一个本地创建的会话，由 `kb_core` 分配标识。
    CreateSession(CreateSessionRequest),

    /// 删除一个会话。
    RemoveSession {
        /// 目标工作区。
        workspace_id: WorkspaceId,

        /// 要删除的会话。
        session_id: SessionId,
    },

    /// 读取一个会话的完整内容（含全部消息）。
    GetSession {
        /// 目标工作区。
        workspace_id: WorkspaceId,

        /// 目标会话。
        session_id: SessionId,
    },

    /// 列出工作区目录下的条目。
    ListDirectory {
        /// 目标工作区。
        workspace_id: WorkspaceId,

        /// 相对工作区根目录的路径；根目录用空串表示。
        relative_path: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试提问请求能往返序列化，且缺省字段被省略。
    ///
    /// - 手段：构造一个没有指定服务的 `Ask`，序列化后反序列化。
    /// - 判断：JSON 以 `{"Ask":` 开头（外部标签）；`service_id` 编码为 `null`
    ///   （不省略）；往返结果与原始值完全相等。
    #[test]
    fn ask_request_round_trips_() {
        let request = Request::Ask(AskRequest {
            workspace_id: WorkspaceId::new("w-1"),
            session_id: SessionId::new("s-1"),
            turn_id: TurnId::new("t-1"),
            question: "你好，介绍一下知识库".to_string(),
            service_id: None,
        });

        let json = serde_json::to_string(&request).expect("应当能序列化");
        assert!(json.starts_with(r#"{"Ask""#), "实际 JSON: {json}");
        assert!(json.contains(r#""service_id":null"#), "实际 JSON: {json}");

        let parsed: Request = serde_json::from_str(&json).expect("应当能反序列化");
        assert_eq!(parsed, request);
    }

    /// 测试本地创建工作区后再同步的请求形态。
    ///
    /// - 手段：构造 `AddWorkspace(AddWorkspaceRequest)`，其中 `local_id` 由
    ///   `LocalId::generate()` 产生。
    /// - 判断：JSON 同时包含 `AddWorkspace` 与 `local_id` 字段；
    ///   往返后 `local_id` 不变——即"客户端临时标识"确实被带到了服务端。
    #[test]
    fn add_workspace_carries_local_id_() {
        let local_id = LocalId::generate();
        let request = Request::AddWorkspace(AddWorkspaceRequest {
            local_id: local_id.clone(),
            name: "笔记".to_string(),
            path: "/home/me/notes".to_string(),
        });

        let json = serde_json::to_string(&request).expect("应当能序列化");
        assert!(json.contains("AddWorkspace"), "实际 JSON: {json}");
        assert!(json.contains("local_id"), "实际 JSON: {json}");

        let parsed: Request = serde_json::from_str(&json).expect("应当能反序列化");
        match parsed {
            Request::AddWorkspace(payload) => assert_eq!(payload.local_id, local_id),
            other => panic!("应当是 AddWorkspace，实际: {other:?}"),
        }
    }

    /// 测试同步会话时可以带上本地已攒的消息。
    ///
    /// - 手段：构造一个 `CreateSession`，其 `turns` 含一条消息。
    /// - 判断：JSON 中出现 `turns`；反序列化后消息条数为 1；
    ///   即便 `turns` 为空也仍然编码为 `[]`（不省略）。
    #[test]
    fn create_session_may_carry_local_turns_() {
        let with_turns = Request::CreateSession(CreateSessionRequest {
            workspace_id: WorkspaceId::new("w-1"),
            local_id: LocalId::new("l-1"),
            title: Some("新会话".to_string()),
            turns: vec![Turn {
                turn_id: TurnId::new("t-1"),
                role: abs_llm::v1::cont::Role::User,
                text: "离线时记的一句".to_string(),
                reasoning: String::new(),
                state: super::super::content_::TurnState::Done,
                tool_calls: Vec::new(),
                usage: None,
                notice: None,
            }],
        });
        let json = serde_json::to_string(&with_turns).expect("应当能序列化");
        assert!(json.contains("turns"), "实际 JSON: {json}");

        let parsed: Request = serde_json::from_str(&json).expect("应当能反序列化");
        match parsed {
            Request::CreateSession(payload) => assert_eq!(payload.turns.len(), 1),
            other => panic!("应当是 CreateSession，实际: {other:?}"),
        }

        let empty = Request::CreateSession(CreateSessionRequest {
            workspace_id: WorkspaceId::new("w-1"),
            local_id: LocalId::new("l-1"),
            title: None,
            turns: Vec::new(),
        });
        let json = serde_json::to_string(&empty).expect("应当能序列化");
        assert!(json.contains(r#""turns":[]"#), "实际 JSON: {json}");
        assert!(json.contains(r#""title":null"#), "实际 JSON: {json}");
    }
}
