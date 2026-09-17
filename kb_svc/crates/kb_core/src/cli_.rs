//! **临时**的 CRUD 子命令。
//!
//! 本阶段 `kb_svc_servo_ipc` 尚未接通；为了能立刻用手检验「工作区 / 会话的
//! 增删查改」，这里把 [`Store`] 的能力直接挂到命令行上。
//!
//! 它与将来的 IPC 路径**共用同一个存储层**，所以现在跑通的行为，就是以后
//! 请求 / 应答要满足的行为。等 IPC 接好后，本模块可以保留为维护命令，
//! 也可以整体删除——**它不构成对外约定**，输出格式只求人眼好读。
//!
//! # 输出格式
//!
//! - 列表类命令每行一条，字段用制表符分隔，最后一行以 `#` 开头给出计数；
//! - `show` 类命令直接打印协议类型的 JSON（见
//!   [`abs_kb_svc::v1::desktop::SessionDetail`]），与存储文件里的内容一致。

use abs_kb_svc::v1::desktop::{SessionId, SessionSummary, Workspace, WorkspaceId};

use crate::args_::{Paths, SessionCommand, WorkspaceCommand};
use crate::store_::{Store, StoreError};

/// 执行一条工作区子命令。
///
/// # Errors
///
/// 透传 [`StoreError`]：标识不合法、目标不存在、落盘失败都会在这里冒出来。
pub async fn run_workspace(paths: &Paths, command: WorkspaceCommand) -> Result<(), StoreError> {
    let store = Store::open(&paths.storage_dir).await?;

    match command {
        WorkspaceCommand::Add { name, path } => {
            let workspace = store.add_workspace(&name, &path).await?;
            println!("{}", format_workspace_(&workspace));
        }
        WorkspaceCommand::List => {
            let list = store.list_workspaces().await?;
            for workspace in &list.workspaces {
                println!("{}", format_workspace_(workspace));
            }
            println!("# 共 {} 个工作区", list.workspaces.len());
        }
        WorkspaceCommand::Show { workspace_id } => {
            let workspace = store.get_workspace(&WorkspaceId::new(workspace_id)).await?;
            println!("{}", format_workspace_(&workspace));
        }
        WorkspaceCommand::Update {
            workspace_id,
            name,
            path,
        } => {
            let id = WorkspaceId::new(workspace_id);
            let mut workspace = store.get_workspace(&id).await?;
            if let Some(name) = name {
                workspace.name = name;
            }
            if let Some(path) = path {
                workspace.path = path;
            }
            store.save_workspace(&workspace).await?;
            println!("{}", format_workspace_(&workspace));
        }
        WorkspaceCommand::Remove { workspace_id } => {
            let id = WorkspaceId::new(workspace_id);
            store.remove_workspace(&id).await?;
            println!("已删除工作区 {id}（连同它名下的会话）");
        }
    }

    Ok(())
}

/// 执行一条会话子命令。
///
/// # Errors
///
/// 透传 [`StoreError`]。
pub async fn run_session(paths: &Paths, command: SessionCommand) -> Result<(), StoreError> {
    let store = Store::open(&paths.storage_dir).await?;

    match command {
        SessionCommand::Add {
            workspace_id,
            title,
        } => {
            let summary = store
                .create_session(&WorkspaceId::new(workspace_id), title, Vec::new())
                .await?;
            println!("{}", format_session_(&summary));
        }
        SessionCommand::List { workspace_id } => {
            let list = store.list_sessions(&WorkspaceId::new(workspace_id)).await?;
            for summary in &list.sessions {
                println!("{}", format_session_(summary));
            }
            println!("# 共 {} 个会话", list.sessions.len());
        }
        SessionCommand::Show {
            workspace_id,
            session_id,
        } => {
            let detail = store
                .get_session(&WorkspaceId::new(workspace_id), &SessionId::new(session_id))
                .await?;
            let json = serde_json::to_string_pretty(&detail).map_err(StoreError::Encode)?;
            println!("{json}");
        }
        SessionCommand::Update {
            workspace_id,
            session_id,
            title,
        } => {
            let summary = store
                .rename_session(
                    &WorkspaceId::new(workspace_id),
                    &SessionId::new(session_id),
                    &title,
                )
                .await?;
            println!("{}", format_session_(&summary));
        }
        SessionCommand::Remove {
            workspace_id,
            session_id,
        } => {
            let id = SessionId::new(session_id);
            store
                .remove_session(&WorkspaceId::new(workspace_id), &id)
                .await?;
            println!("已删除会话 {id}");
        }
    }

    Ok(())
}

/// 工作区的一行输出：`<标识>\t<名字>\t<路径>`。
fn format_workspace_(workspace: &Workspace) -> String {
    format!(
        "{}\t{}\t{}",
        workspace.workspace_id, workspace.name, workspace.path
    )
}

/// 会话摘要的一行输出：`<标识>\t<标题>\t<条数> 条\t<活动时间>`。
///
/// 活动时间保持协议里的"自 Unix 纪元起的毫秒数"原样输出，不做本地化格式化：
/// 本模块是调试入口，少一处时区逻辑就少一处出错的地方。
fn format_session_(summary: &SessionSummary) -> String {
    format!(
        "{}\t{}\t{} 条\t更新于 {}",
        summary.session_id, summary.title, summary.turn_count, summary.updated_at_millis
    )
}
