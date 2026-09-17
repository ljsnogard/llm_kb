//! 工作区与会话：由 `kb_core` 持有、多端同步的对象。
//!
//! # 归属与标识
//!
//! 工作区与会话都是 **`kb_core` 管理的服务端状态**，因此：
//!
//! - 每个工作区有自己的 [`WorkspaceId`]、每个会话有自己的 [`SessionId`]，**由 `kb_core` 分配**；
//! - 任何客户端都可以**先在本地创建**它们：此时它只能拿到 [`LocalId`]，
//!   直到同步（[`Request::AddWorkspace`](crate::v1::desktop::Request::AddWorkspace)
//!   / [`Request::CreateSession`](crate::v1::desktop::Request::CreateSession)）时
//!   才由 `kb_core` 分配真正的标识；
//! - 同步的顺序是**先工作区、后会话**：`CreateSession` 需要 `workspace_id`，
//!   而那个标识只能来自工作区同步的结果。
//!
//! # 为什么是扁平结构
//!
//! 与客户端的 `Workspace`（内嵌 `sessions`）不同，协议里的 [`Workspace`] **不含**会话。
//! 会话通过 [`Request::ListSessions`](crate::v1::desktop::Request::ListSessions) 按需拉取，
//! 避免每次列工作区都把整棵历史树传一遍；"工作区 → 会话 → 消息"的树由客户端自己组装。
//!
//! # "当前选中"不属于这里
//!
//! "当前停在哪个工作区 / 哪个会话"是**每个客户端各自的界面状态**（多窗口时会不同），
//! 因此不上传。相反，"当前生效的 LLM 服务"是服务端状态，见
//! [`ServerState::active_service`](crate::v1::desktop::ServerState::active_service)。

use serde::{Deserialize, Serialize};

use super::content_::Turn;
use super::ids_::{SessionId, WorkspaceId};

/// 一个知识库工作区。
///
/// 对应磁盘上的一个目录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    /// 工作区标识，由 `kb_core` 分配。
    pub workspace_id: WorkspaceId,

    /// 展示名。
    pub name: String,

    /// 对应的磁盘目录。
    pub path: String,
}

/// 工作区列表。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceList {
    /// 全部工作区。
    pub workspaces: Vec<Workspace>,
}

/// 一个会话的摘要（不含消息正文）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    /// 会话标识，由 `kb_core` 分配。
    pub session_id: SessionId,

    /// 所属工作区。
    pub workspace_id: WorkspaceId,

    /// 会话标题（通常取首条用户消息的前若干字）。
    pub title: String,

    /// 最近一次活动时间（自 Unix 纪元起的毫秒数）。
    pub updated_at_millis: i64,

    /// 会话内的消息条数，供列表页展示。
    pub turn_count: u32,
}

/// 一个会话的完整内容。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionDetail {
    /// 会话摘要。
    pub summary: SessionSummary,

    /// 会话内的全部消息，按时间顺序。
    pub turns: Vec<Turn>,
}

/// 会话列表。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionList {
    /// 某个工作区下的全部会话。
    pub sessions: Vec<SessionSummary>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1::desktop::{LocalId, TurnState};

    /// 测试工作区同步的"请求带临时标识、应答带真实标识"这一配对关系。
    ///
    /// - 手段：模拟客户端本地创建一个工作区（只有 `LocalId`），
    ///   把 `LocalId` 与 `kb_core` 分配的 `Workspace` 放在一起序列化。
    /// - 判断：两边的字符串都完整保留，且二者不同——
    ///   这支撑"同步后把 `LocalId` 换成 `WorkspaceId`"这一步是有据可依的。
    #[test]
    fn local_id_pairs_with_assigned_workspace_() {
        let local_id = LocalId::generate();
        let workspace = Workspace {
            workspace_id: WorkspaceId::generate(),
            name: "笔记".to_string(),
            path: "/home/me/notes".to_string(),
        };

        let json =
            serde_json::to_string(&(local_id.clone(), workspace.clone())).expect("应当能序列化");
        let parsed: (LocalId, Workspace) = serde_json::from_str(&json).expect("应当能反序列化");

        assert_eq!(parsed.0, local_id);
        assert_eq!(parsed.1, workspace);
        assert_ne!(
            local_id.as_str().to_string(),
            parsed.1.workspace_id.as_str().to_string(),
            "本地临时标识与服务端标识不应相同"
        );
    }

    /// 测试会话详情能带上本地创建时就已存在的消息。
    ///
    /// - 手段：构造一个含两条消息的 `SessionDetail`，序列化后反序列化。
    /// - 判断：`turns` 的两条消息都保留，`turn_count` 为 2；
    ///   这支撑"客户端离线先攒消息、同步时一起交给 kb_core"。
    #[test]
    fn session_detail_carries_locally_created_turns_() {
        let detail = SessionDetail {
            summary: SessionSummary {
                session_id: SessionId::generate(),
                workspace_id: WorkspaceId::generate(),
                title: "新会话".to_string(),
                updated_at_millis: 1_760_000_000_000,
                turn_count: 2,
            },
            turns: vec![
                Turn {
                    turn_id: "t-1".into(),
                    role: abs_llm::v1::cont::Role::User,
                    text: "你好".to_string(),
                    reasoning: String::new(),
                    state: TurnState::Done,
                    tool_calls: Vec::new(),
                    usage: None,
                    notice: None,
                },
                Turn {
                    turn_id: "t-2".into(),
                    role: abs_llm::v1::cont::Role::Assistant,
                    text: "你好，有什么可以帮你".to_string(),
                    reasoning: String::new(),
                    state: TurnState::Done,
                    tool_calls: Vec::new(),
                    usage: None,
                    notice: None,
                },
            ],
        };

        let json = serde_json::to_string(&detail).expect("应当能序列化");
        let parsed: SessionDetail = serde_json::from_str(&json).expect("应当能反序列化");

        assert_eq!(parsed.turns.len(), 2);
        assert_eq!(parsed.summary.turn_count, 2);
        assert_eq!(parsed, detail);
    }
}
