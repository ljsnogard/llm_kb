//! `kb_core` → 客户端的应答。
//!
//! 每个变体与 [`Request`](crate::v1::desktop::Request) 一一对应；
//! [`Reply::Ack`] 用于"成功但没有内容"的请求（删除、切换生效服务等）。
//! 业务错误走 [`Reply::Error`]。

use serde::{Deserialize, Serialize};

use super::error_::ErrorReply;
use super::fs_::DirectoryListing;
use super::handshake_::ServerInfo;
use super::ids_::LocalId;
use super::service_::{ServiceList, ServiceSummary};
use super::workspace_::{SessionDetail, SessionList, SessionSummary, Workspace, WorkspaceList};

/// `kb_core` 对某个请求的应答。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Reply {
    /// [`Request::Hello`](crate::v1::desktop::Request::Hello) 的应答。
    Hello(ServerInfo),

    /// 成功，且没有需要返回的内容。
    Ack,

    /// 服务列表（列出服务之后，以及服务配置变化之后）。
    ServiceList(ServiceList),

    /// 某个服务被新增或覆盖。
    ServiceUpdated(ServiceSummary),

    /// 工作区列表。
    WorkspaceList(WorkspaceList),

    /// 工作区已建立。
    ///
    /// 携带 `local_id` 是为了让客户端知道"这是你刚才用哪个临时标识提交的那个"，
    /// 从而把本地对象换成服务端分配的标识。
    WorkspaceAdded {
        /// 客户端提交时使用的临时标识。
        local_id: LocalId,

        /// 服务端建立的工作区（含 `kb_core` 分配的标识）。
        workspace: Workspace,
    },

    /// 会话列表。
    SessionList(SessionList),

    /// 会话已建立。
    ///
    /// 与 [`Reply::WorkspaceAdded`] 同理，携带 `local_id` 用于替换本地标识。
    SessionCreated {
        /// 客户端提交时使用的临时标识。
        local_id: LocalId,

        /// 服务端建立的会话摘要（含 `kb_core` 分配的标识）。
        session: SessionSummary,
    },

    /// 会话的完整内容。
    SessionDetail(SessionDetail),

    /// 目录列举结果。
    DirectoryListing(DirectoryListing),

    /// 业务错误。
    Error(ErrorReply),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1::desktop::{ErrorCode, SessionId, WorkspaceId};

    /// 测试"本地创建 → 服务端分配标识"的配对在应答里可还原。
    ///
    /// - 手段：构造 `WorkspaceAdded` 与 `SessionCreated` 两个应答并往返序列化。
    /// - 判断：`local_id` 与分配到的标识都被保留，且 `WorkspaceAdded` /
    ///   `SessionCreated` 作为外部标签出现在 JSON 里。
    #[test]
    fn sync_replies_pair_local_id_with_assigned_id_() {
        let workspace_reply = Reply::WorkspaceAdded {
            local_id: LocalId::new("l-1"),
            workspace: Workspace {
                workspace_id: WorkspaceId::new("w-1"),
                name: "笔记".to_string(),
                path: "/home/me/notes".to_string(),
            },
        };
        let json = serde_json::to_string(&workspace_reply).expect("应当能序列化");
        assert!(
            json.starts_with(r#"{"WorkspaceAdded""#),
            "实际 JSON: {json}"
        );
        let parsed: Reply = serde_json::from_str(&json).expect("应当能反序列化");
        assert_eq!(parsed, workspace_reply);

        let session_reply = Reply::SessionCreated {
            local_id: LocalId::new("l-2"),
            session: SessionSummary {
                session_id: SessionId::new("s-1"),
                workspace_id: WorkspaceId::new("w-1"),
                title: "新会话".to_string(),
                updated_at_millis: 1_760_000_000_000,
                turn_count: 0,
            },
        };
        let json = serde_json::to_string(&session_reply).expect("应当能序列化");
        assert!(
            json.starts_with(r#"{"SessionCreated""#),
            "实际 JSON: {json}"
        );
        let parsed: Reply = serde_json::from_str(&json).expect("应当能反序列化");
        assert_eq!(parsed, session_reply);
    }

    /// 测试业务错误应答能往返序列化。
    ///
    /// - 手段：构造 `Reply::Error`，序列化后检查字符串并反序列化。
    /// - 判断：JSON 以 `{"Error":` 开头且含 `"code":"not_found"`；往返相等。
    #[test]
    fn error_reply_round_trips_() {
        let reply = Reply::Error(ErrorReply {
            code: ErrorCode::NotFound,
            message: "工作区不存在".to_string(),
        });

        let json = serde_json::to_string(&reply).expect("应当能序列化");
        assert!(json.starts_with(r#"{"Error""#), "实际 JSON: {json}");
        assert!(json.contains(r#""code":"not_found""#), "实际 JSON: {json}");

        let parsed: Reply = serde_json::from_str(&json).expect("应当能反序列化");
        assert_eq!(parsed, reply);
    }
}
