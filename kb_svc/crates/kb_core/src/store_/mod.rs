//! 工作区与会话的**本地文件存储**。
//!
//! # 为什么暂时是本地文件
//!
//! `kb_core` 最终的载体是 Turso 数据库，但数据库还没接。为了让
//! 「工作区 / 会话的增删查改」这条业务链路先能跑起来并接受检验，本阶段用本地
//! 文件代替数据库。**替换的边界就是本模块**：语义（谁存在、谁属于谁、
//! 删除是否级联）保持不变，换实现时只替换 [`Store`]。
//!
//! # 布局（对外约定）
//!
//! ```text
//! <storage_dir>/
//! ├── workspaces/
//! │   └── <workspace_id>.json        一个工作区（`Workspace`）
//! └── sessions/
//!     └── <workspace_id>/
//!         └── <session_id>.json      一个会话（`SessionDetail`：摘要 + 全部消息）
//! ```
//!
//! 文件内容就是 `abs_kb_svc::v1::desktop` 中协议类型的 JSON 表示——
//! **存储格式与线上格式同源**，不引入第二套结构，因此不会出现"能存进去、
//! 却发不出去"的字段。
//!
//! # 三条硬性约定
//!
//! 1. **标识必须能安全地当文件名**：只允许 ASCII 字母、数字、`-`、`_`，且不超过
//!    128 字节。协议把标识当作不透明字符串，因此不能假定它一定由
//!    `generate()` 产生；含 `../` 的标识会在碰盘之前就被拒绝。
//! 2. **写入先落临时文件再 `rename`**：读到的内容要么是旧的、要么是新的，
//!    不会是半个 JSON。进程被强杀时最坏留下一个 `*.json.tmp`。
//! 3. **会话属于工作区**：`create_session` / `list_sessions` 会先确认工作区存在；
//!    删除工作区会级联删除它的会话目录。工作区不存在时 `list_sessions`
//!    **返回错误而不是空列表**——否则客户端把标识写错时会误以为"这里没有会话"。
//!
//! # 每个操作都是可取消的
//!
//! 本模块**没有手写的对外 `async fn`**：每个操作都由
//! [`gen_mcf2::gen_may_cancel_future`] 展开出一对类型——`XxxAsync`
//! （提供 `IntoFuture`，即不可取消路径）与配套的 `XxxFuture`
//! （`.may_cancel_with(token)` 之后的可取消路径）：
//!
//! ```text
//! store.list_workspaces().await?                      // 不可取消
//! store.list_workspaces().may_cancel_with(t).await?   // 可取消
//! ```
//!
//! 取消的落点在 [`race_cancel_`]：**整个操作体**与取消信号赛跑，令牌先触发就
//! 返回 [`StoreError::Cancelled`]，尚未完成的等待随 future 一起被丢弃。
//! "丢弃"对 `compio::fs` 是真的取消，对 `spawn_blocking` 里的阻塞任务则只是
//! **不再等它**——代价换来的是调用方不会被挂住。
//!
//! ## 内部辅助函数为什么不各自再包一层宏
//!
//! [`create_dir_all_`] / [`read_json_`] / [`write_json_`] 这些私有辅助**只在上面
//! 那些已经被取消令牌包住的操作体里**被调用：它们的等待会随外层 future 被丢弃
//! 而放弃，因此不再各自生成一套 future 类型（没有第二个调用者需要它）。
//! 它们**没有**假定"调用者不会取消"，只是把这件事交给唯一的外层统一处理。
//! 唯一一个不 await 任何东西的取消判断被提成了 [`cancel_guard_`]。

mod error_;
mod layout_;

pub use error_::StoreError;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::task::Poll;

use abs_cancel::TrCancellationToken;
use abs_kb_svc::v1::desktop::{
    SessionDetail, SessionId, SessionList, SessionSummary, Turn, Workspace, WorkspaceId,
    WorkspaceList,
};
use abs_llm::v1::cont::Role;
use gen_mcf2::gen_may_cancel_future;
use layout_::{
    check_id_, session_file_, sessions_dir_, sessions_root_, workspace_file_, workspaces_dir_,
};
use log::debug;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// 会话标题的缺省推导上限（字符数，不是字节数）。
const TITLE_MAX_CHARS_: usize = 24;

/// 缺省会话标题。
const DEFAULT_TITLE_: &str = "新会话";

/// 工作区与会话的本地文件存储。
///
/// 一个实例对应一个存储根目录；创建后可以安全地在多个异步任务之间共享引用
/// （内部没有可变状态，所有一致性都落在文件系统上）。
#[derive(Debug, Clone)]
pub struct Store {
    /// 存储根目录。
    root_: PathBuf,
}

/// 每个操作都返回一个 `gen_mcf2` 展开的可取消 future，而不是裸 `async fn`。
///
/// 这些构造器本身是**同步**的：它们只负责把参数装进 future，真正的等待发生在
/// `.await` 或 `.may_cancel_with(token).await` 上。
impl Store {
    /// 打开（必要时创建）一个存储根目录。
    ///
    /// 会顺手建好 `workspaces/` 与 `sessions/`，这样"目录树长什么样"在第一次
    /// 启动后就能直接看到，不必等第一个对象被写入。
    pub fn open<'r>(root: &'r Path) -> OpenAsync<'r, 'r> {
        OpenAsync::new(root)
    }

    /// 存储根目录。
    pub fn root(&self) -> &Path {
        &self.root_
    }

    // ── 工作区 ──────────────────────────────────────────────────────────

    /// 列出全部工作区，按标识升序。
    pub fn list_workspaces<'f>(&'f self) -> ListWorkspacesAsync<'f, 'f> {
        ListWorkspacesAsync::new(self)
    }

    /// 读取一个工作区。
    pub fn get_workspace<'f>(&'f self, workspace_id: &'f WorkspaceId) -> GetWorkspaceAsync<'f, 'f> {
        GetWorkspaceAsync::new(self, workspace_id)
    }

    /// 新建一个工作区，标识由本进程生成。
    ///
    /// 返回的对象里带着新分配的 [`WorkspaceId`]；IPC 层负责把它连同客户端提交的
    /// `LocalId` 一起回给客户端。
    pub fn add_workspace<'f>(&'f self, name: &'f str, path: &'f str) -> AddWorkspaceAsync<'f, 'f> {
        AddWorkspaceAsync::new(self, name, path)
    }

    /// 写入（覆盖）一个工作区。
    ///
    /// 这是工作区的"改"：重命名、改路径都走这里。它是**原样落盘**的，
    /// 不做任何字段推导。
    pub fn save_workspace<'f>(&'f self, workspace: &'f Workspace) -> SaveWorkspaceAsync<'f, 'f> {
        SaveWorkspaceAsync::new(self, workspace)
    }

    /// 删除一个工作区，并级联删除它名下的全部会话。
    ///
    /// 级联是刻意的：会话的归属字段就是 `workspace_id`，工作区没了，
    /// 这些会话既列不出来也读不出来，留着只会变成垃圾。
    pub fn remove_workspace<'f>(
        &'f self,
        workspace_id: &'f WorkspaceId,
    ) -> RemoveWorkspaceAsync<'f, 'f> {
        RemoveWorkspaceAsync::new(self, workspace_id)
    }

    // ── 会话 ────────────────────────────────────────────────────────────

    /// 列出某个工作区下的全部会话摘要，按最近活动时间降序。
    ///
    /// 列表里**不含消息正文**：正文通过 [`Store::get_session`] 按需拉取，
    /// 这与协议里"只发差异、不整棵树"的约定一致。
    pub fn list_sessions<'f>(&'f self, workspace_id: &'f WorkspaceId) -> ListSessionsAsync<'f, 'f> {
        ListSessionsAsync::new(self, workspace_id)
    }

    /// 读取一个会话的完整内容（摘要 + 全部消息）。
    pub fn get_session<'f>(
        &'f self,
        workspace_id: &'f WorkspaceId,
        session_id: &'f SessionId,
    ) -> GetSessionAsync<'f, 'f> {
        GetSessionAsync::new(self, workspace_id, session_id)
    }

    /// 在某个工作区下新建一个会话，标识由本进程生成。
    ///
    /// `title` 为 `None` 或空白时，从 `turns` 里第一条用户消息推导；
    /// `turns` 是客户端在离线期间攒下的历史，通常为空。
    pub fn create_session<'f>(
        &'f self,
        workspace_id: &'f WorkspaceId,
        title: Option<String>,
        turns: Vec<Turn>,
    ) -> CreateSessionAsync<'f, 'f> {
        CreateSessionAsync::new(self, workspace_id, title, turns)
    }

    /// 写入（覆盖）一个会话。
    ///
    /// **原样落盘**：调用方要自己保证摘要与 `turns` 一致。只是改标题/追加消息时，
    /// 用 [`Store::rename_session`] 与 [`Store::append_turns`] 更安全，
    /// 它们会顺手刷新 `turn_count` 与 `updated_at_millis`。
    pub fn save_session<'f>(&'f self, detail: &'f SessionDetail) -> SaveSessionAsync<'f, 'f> {
        SaveSessionAsync::new(self, detail)
    }

    /// 重命名一个会话，并刷新它的活动时间。这是会话的"改"。
    pub fn rename_session<'f>(
        &'f self,
        workspace_id: &'f WorkspaceId,
        session_id: &'f SessionId,
        title: &'f str,
    ) -> RenameSessionAsync<'f, 'f> {
        RenameSessionAsync::new(self, workspace_id, session_id, title)
    }

    /// 向一个会话追加若干条消息，并同步刷新摘要。
    ///
    /// 这是「提问 → 增量 → 落库」链路要用到的写入口：摘要里的 `turn_count` 与
    /// `updated_at_millis` 由本方法维护，调用方不必操心。
    // 目前只有单元测试在调用；接上生成域之后就会进入请求处理路径。
    #[allow(dead_code)]
    pub fn append_turns<'f>(
        &'f self,
        workspace_id: &'f WorkspaceId,
        session_id: &'f SessionId,
        turns: &'f [Turn],
    ) -> AppendTurnsAsync<'f, 'f> {
        AppendTurnsAsync::new(self, workspace_id, session_id, turns)
    }

    /// 删除一个会话。
    pub fn remove_session<'f>(
        &'f self,
        workspace_id: &'f WorkspaceId,
        session_id: &'f SessionId,
    ) -> RemoveSessionAsync<'f, 'f> {
        RemoveSessionAsync::new(self, workspace_id, session_id)
    }
}

// ============================================================================
// 操作体：宏只能作用于模块级自由函数，所以每个操作在这里展开
// ============================================================================

/// [`Store::open`] 的操作体。
#[gen_may_cancel_future(Open, pub)]
pub async fn open_async<'r, C>(root: &'r Path, cancel: C) -> Result<Store, StoreError>
where
    C: TrCancellationToken,
{
    cancel_guard_(&cancel)?;
    race_cancel_(
        async move {
            create_dir_all_(root).await?;
            create_dir_all_(&workspaces_dir_(root)).await?;
            create_dir_all_(&sessions_root_(root)).await?;
            Ok(Store {
                root_: root.to_path_buf(),
            })
        },
        cancel,
    )
    .await
}

/// [`Store::list_workspaces`] 的操作体。
#[gen_may_cancel_future(ListWorkspaces, pub)]
pub async fn list_workspaces_async<'s, C>(
    store: &'s Store,
    cancel: C,
) -> Result<WorkspaceList, StoreError>
where
    C: TrCancellationToken,
{
    cancel_guard_(&cancel)?;
    race_cancel_(
        async move {
            let files = list_json_files_(&workspaces_dir_(&store.root_)).await?;
            let mut workspaces = Vec::with_capacity(files.len());
            for file in files {
                workspaces
                    .push(read_json_::<Workspace>(&file, "工作区", &file_stem_(&file)).await?);
            }
            workspaces.sort_by(|left, right| left.workspace_id.cmp(&right.workspace_id));
            Ok(WorkspaceList { workspaces })
        },
        cancel,
    )
    .await
}

/// [`Store::get_workspace`] 的操作体。
#[gen_may_cancel_future(GetWorkspace, pub)]
pub async fn get_workspace_async<'s, C>(
    store: &'s Store,
    workspace_id: &'s WorkspaceId,
    cancel: C,
) -> Result<Workspace, StoreError>
where
    C: TrCancellationToken,
{
    cancel_guard_(&cancel)?;
    race_cancel_(
        async move {
            check_id_("工作区", workspace_id.as_str())?;
            let path = workspace_file_(&store.root_, workspace_id.as_str());
            read_json_(&path, "工作区", workspace_id.as_str()).await
        },
        cancel,
    )
    .await
}

/// [`Store::add_workspace`] 的操作体。
#[gen_may_cancel_future(AddWorkspace, pub)]
pub async fn add_workspace_async<'s, C>(
    store: &'s Store,
    name: &'s str,
    path: &'s str,
    cancel: C,
) -> Result<Workspace, StoreError>
where
    C: TrCancellationToken,
{
    cancel_guard_(&cancel)?;
    race_cancel_(
        async move {
            let workspace = Workspace {
                workspace_id: WorkspaceId::generate(),
                name: name.to_string(),
                path: path.to_string(),
            };
            store.save_workspace(&workspace).await?;
            Ok(workspace)
        },
        cancel,
    )
    .await
}

/// [`Store::save_workspace`] 的操作体。
#[gen_may_cancel_future(SaveWorkspace, pub)]
pub async fn save_workspace_async<'s, C>(
    store: &'s Store,
    workspace: &'s Workspace,
    cancel: C,
) -> Result<(), StoreError>
where
    C: TrCancellationToken,
{
    cancel_guard_(&cancel)?;
    race_cancel_(
        async move {
            check_id_("工作区", workspace.workspace_id.as_str())?;
            let path = workspace_file_(&store.root_, workspace.workspace_id.as_str());
            write_json_(&path, workspace).await
        },
        cancel,
    )
    .await
}

/// [`Store::remove_workspace`] 的操作体。
#[gen_may_cancel_future(RemoveWorkspace, pub)]
pub async fn remove_workspace_async<'s, C>(
    store: &'s Store,
    workspace_id: &'s WorkspaceId,
    cancel: C,
) -> Result<(), StoreError>
where
    C: TrCancellationToken,
{
    cancel_guard_(&cancel)?;
    race_cancel_(
        async move {
            check_id_("工作区", workspace_id.as_str())?;

            // 顺序：先删会话目录，再删工作区文件。反过来会留下"工作区已消失、
            // 会话目录还在"的孤儿状态；按当前顺序中断，最坏也只是工作区被回退成
            // "存在但没有会话"，仍然自洽。
            remove_dir_all_(&sessions_dir_(&store.root_, workspace_id.as_str())).await?;
            remove_file_checked_(
                &workspace_file_(&store.root_, workspace_id.as_str()),
                "工作区",
                workspace_id.as_str(),
            )
            .await
        },
        cancel,
    )
    .await
}

/// [`Store::list_sessions`] 的操作体。
#[gen_may_cancel_future(ListSessions, pub)]
pub async fn list_sessions_async<'s, C>(
    store: &'s Store,
    workspace_id: &'s WorkspaceId,
    cancel: C,
) -> Result<SessionList, StoreError>
where
    C: TrCancellationToken,
{
    cancel_guard_(&cancel)?;
    race_cancel_(
        async move {
            store.get_workspace(workspace_id).await?;

            let files =
                list_json_files_(&sessions_dir_(&store.root_, workspace_id.as_str())).await?;
            let mut sessions = Vec::with_capacity(files.len());
            for file in files {
                let detail = read_json_::<SessionDetail>(&file, "会话", &file_stem_(&file)).await?;
                sessions.push(detail.summary);
            }
            sessions.sort_by(|left, right| {
                right
                    .updated_at_millis
                    .cmp(&left.updated_at_millis)
                    .then_with(|| left.session_id.cmp(&right.session_id))
            });
            Ok(SessionList { sessions })
        },
        cancel,
    )
    .await
}

/// [`Store::get_session`] 的操作体。
#[gen_may_cancel_future(GetSession, pub)]
pub async fn get_session_async<'s, C>(
    store: &'s Store,
    workspace_id: &'s WorkspaceId,
    session_id: &'s SessionId,
    cancel: C,
) -> Result<SessionDetail, StoreError>
where
    C: TrCancellationToken,
{
    cancel_guard_(&cancel)?;
    race_cancel_(
        async move {
            check_id_("工作区", workspace_id.as_str())?;
            check_id_("会话", session_id.as_str())?;
            let path = session_file_(&store.root_, workspace_id.as_str(), session_id.as_str());
            read_json_(&path, "会话", session_id.as_str()).await
        },
        cancel,
    )
    .await
}

/// [`Store::create_session`] 的操作体。
#[gen_may_cancel_future(CreateSession, pub)]
pub async fn create_session_async<'s, C>(
    store: &'s Store,
    workspace_id: &'s WorkspaceId,
    title: Option<String>,
    turns: Vec<Turn>,
    cancel: C,
) -> Result<SessionSummary, StoreError>
where
    C: TrCancellationToken,
{
    cancel_guard_(&cancel)?;
    race_cancel_(
        async move {
            store.get_workspace(workspace_id).await?;

            let summary = SessionSummary {
                session_id: SessionId::generate(),
                workspace_id: workspace_id.clone(),
                title: normalize_title_(title, &turns),
                updated_at_millis: now_millis_(),
                turn_count: turn_count_of_(&turns),
            };
            let detail = SessionDetail {
                summary: summary.clone(),
                turns,
            };
            store.save_session(&detail).await?;
            Ok(summary)
        },
        cancel,
    )
    .await
}

/// [`Store::save_session`] 的操作体。
#[gen_may_cancel_future(SaveSession, pub)]
pub async fn save_session_async<'s, C>(
    store: &'s Store,
    detail: &'s SessionDetail,
    cancel: C,
) -> Result<(), StoreError>
where
    C: TrCancellationToken,
{
    cancel_guard_(&cancel)?;
    race_cancel_(
        async move {
            check_id_("工作区", detail.summary.workspace_id.as_str())?;
            check_id_("会话", detail.summary.session_id.as_str())?;

            let dir = sessions_dir_(&store.root_, detail.summary.workspace_id.as_str());
            create_dir_all_(&dir).await?;
            let path = session_file_(
                &store.root_,
                detail.summary.workspace_id.as_str(),
                detail.summary.session_id.as_str(),
            );
            write_json_(&path, detail).await
        },
        cancel,
    )
    .await
}

/// [`Store::rename_session`] 的操作体。
#[gen_may_cancel_future(RenameSession, pub)]
pub async fn rename_session_async<'s, C>(
    store: &'s Store,
    workspace_id: &'s WorkspaceId,
    session_id: &'s SessionId,
    title: &'s str,
    cancel: C,
) -> Result<SessionSummary, StoreError>
where
    C: TrCancellationToken,
{
    cancel_guard_(&cancel)?;
    race_cancel_(
        async move {
            let mut detail = store.get_session(workspace_id, session_id).await?;
            detail.summary.title = normalize_title_(Some(title.to_string()), &detail.turns);
            detail.summary.updated_at_millis = now_millis_();
            store.save_session(&detail).await?;
            Ok(detail.summary)
        },
        cancel,
    )
    .await
}

/// [`Store::append_turns`] 的操作体。
#[gen_may_cancel_future(AppendTurns, pub)]
pub async fn append_turns_async<'s, C>(
    store: &'s Store,
    workspace_id: &'s WorkspaceId,
    session_id: &'s SessionId,
    turns: &'s [Turn],
    cancel: C,
) -> Result<SessionSummary, StoreError>
where
    C: TrCancellationToken,
{
    cancel_guard_(&cancel)?;
    race_cancel_(
        async move {
            let mut detail = store.get_session(workspace_id, session_id).await?;
            detail.turns.extend_from_slice(turns);
            detail.summary.turn_count = turn_count_of_(&detail.turns);
            detail.summary.updated_at_millis = now_millis_();
            store.save_session(&detail).await?;
            Ok(detail.summary)
        },
        cancel,
    )
    .await
}

/// [`Store::remove_session`] 的操作体。
#[gen_may_cancel_future(RemoveSession, pub)]
pub async fn remove_session_async<'s, C>(
    store: &'s Store,
    workspace_id: &'s WorkspaceId,
    session_id: &'s SessionId,
    cancel: C,
) -> Result<(), StoreError>
where
    C: TrCancellationToken,
{
    cancel_guard_(&cancel)?;
    race_cancel_(
        async move {
            check_id_("工作区", workspace_id.as_str())?;
            check_id_("会话", session_id.as_str())?;
            remove_file_checked_(
                &session_file_(&store.root_, workspace_id.as_str(), session_id.as_str()),
                "会话",
                session_id.as_str(),
            )
            .await
        },
        cancel,
    )
    .await
}

// ============================================================================
// 取消
// ============================================================================

/// 已经取消就立刻收手，不去碰磁盘。
///
/// 这是本模块里唯一"直接返回一个结果"的取消判断——它不 await 任何东西，
/// 因此不需要可取消 future；放在操作体最前面是为了别白干一趟。
fn cancel_guard_<C>(cancel: &C) -> Result<(), StoreError>
where
    C: TrCancellationToken,
{
    if cancel.is_cancelled() {
        Err(StoreError::Cancelled)
    } else {
        Ok(())
    }
}

/// 让**整个操作**与取消信号赛跑：令牌先触发就放弃尚未完成的等待。
///
/// 之所以在操作的最外层赛一次、而不是在每个 await 点各赛一次：
///
/// - 语义更简单——"这次调用被取消了"只可能产生一个结果；
/// - 覆盖更全——操作体内部的私有辅助（`read_json_` / `write_json_` /
///   `spawn_blocking` 等）不需要各自携带令牌，它们的等待会随这里被丢弃；
/// - 与 `gen_mcf2` 的分工一致：宏负责产出可取消的 future 类型，本函数负责
///   在令牌触发时把它结束掉。
///
/// 代价：操作是分步的，取消可能停在中间（例如已经写了 `.tmp` 但还没 `rename`）。
/// 落盘约定（先写临时文件再 `rename`）保证了这种中间态不会被当成对象读出来。
async fn race_cancel_<T, F, C>(work: F, cancel: C) -> Result<T, StoreError>
where
    F: Future<Output = Result<T, StoreError>>,
    C: TrCancellationToken,
{
    let mut work = std::pin::pin!(work);
    let cancellation = cancel.cancellation();
    let mut cancellation = std::pin::pin!(cancellation);

    std::future::poll_fn(|context| {
        if let Poll::Ready(outcome) = work.as_mut().poll(context) {
            return Poll::Ready(outcome);
        }
        if cancellation.as_mut().poll(context).is_ready() {
            return Poll::Ready(Err(StoreError::Cancelled));
        }
        Poll::Pending
    })
    .await
}

// ── 内部辅助 ────────────────────────────────────────────────────────────

/// 创建目录（含各级父目录）。
async fn create_dir_all_(path: &Path) -> Result<(), StoreError> {
    compio::fs::create_dir_all(path)
        .await
        .map_err(|source| io_error_(path, source))
}

/// 删除文件，把"本来就不存在"翻译成 [`StoreError::NotFound`]。
async fn remove_file_checked_(path: &Path, kind: &'static str, id: &str) -> Result<(), StoreError> {
    match compio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Err(StoreError::NotFound {
            kind,
            id: id.to_string(),
        }),
        Err(source) => Err(io_error_(path, source)),
    }
}

/// 递归删除目录。
///
/// `compio` 0.19 的 `fs` 只提供单层 `remove_dir`，没有 `remove_dir_all`；
/// 目录遍历也没有异步版本。这些操作本身很快，因此交给 compio 的阻塞线程池，
/// **不占用执行器线程**——这正是 `abs_kb_svc` README §5 契约要求的做法。
async fn remove_dir_all_(path: &Path) -> Result<(), StoreError> {
    let path = path.to_path_buf();
    let reported = path.clone();

    compio::runtime::spawn_blocking(move || match std::fs::remove_dir_all(&path) {
        Ok(()) => Ok(()),
        // 目录不存在视为删除成功：删除是幂等的。
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(source),
    })
    .await
    .map_err(|error| StoreError::BlockingTask(error.to_string()))?
    .map_err(|source| io_error_(&reported, source))
}

/// 列出一个目录下的全部 `*.json` 文件，按路径排序。
///
/// 目录不存在时返回空列表（"还没有任何对象"与"目录还没建"对调用方是同一件事）。
async fn list_json_files_(dir: &Path) -> Result<Vec<PathBuf>, StoreError> {
    let dir = dir.to_path_buf();
    let reported = dir.clone();

    let files = compio::runtime::spawn_blocking(move || -> std::io::Result<Vec<PathBuf>> {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(source),
        };

        let mut files = Vec::new();
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                files.push(path);
            }
        }
        // `read_dir` 的顺序由文件系统决定；排序让输出可复现。
        files.sort();
        Ok(files)
    })
    .await
    .map_err(|error| StoreError::BlockingTask(error.to_string()))?
    .map_err(|source| io_error_(&reported, source))?;

    Ok(files)
}

/// 读取一个 JSON 文件。
async fn read_json_<T>(path: &Path, kind: &'static str, id: &str) -> Result<T, StoreError>
where
    T: DeserializeOwned,
{
    let bytes = match compio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(StoreError::NotFound {
                kind,
                id: id.to_string(),
            });
        }
        Err(source) => return Err(io_error_(path, source)),
    };

    serde_json::from_slice(&bytes).map_err(|source| StoreError::Decode {
        path: path.to_path_buf(),
        source,
    })
}

/// 原子地写入一个 JSON 文件。
///
/// 先写到同目录的 `<名字>.json.tmp`，再 `rename` 覆盖目标。`rename` 在同一文件
/// 系统内是原子的，因此读者不会看到半个文件；进程被强杀时最坏留下一个 `.tmp`，
/// 它不会被 [`list_json_files_`] 认作对象。
async fn write_json_<T>(path: &Path, value: &T) -> Result<(), StoreError>
where
    T: Serialize,
{
    let bytes = serde_json::to_vec_pretty(value).map_err(StoreError::Encode)?;
    let temp = temp_path_(path);

    // `compio::fs::write` 返回 `BufResult(io::Result<()>, 原缓冲区)`，缓冲区随
    // 调用被移出再移回；这里只关心前半截。
    compio::fs::write(&temp, bytes)
        .await
        .0
        .map_err(|source| io_error_(&temp, source))?;

    if let Err(source) = compio::fs::rename(&temp, path).await {
        // rename 失败时尽力清掉临时文件，避免留下垃圾；清理失败不覆盖原错误。
        let _ = compio::fs::remove_file(&temp).await;
        return Err(io_error_(path, source));
    }

    debug!("已写入 {}", path.display());
    Ok(())
}

/// 临时文件路径：`<名字>.json` → `<名字>.json.tmp`。
fn temp_path_(path: &Path) -> PathBuf {
    path.with_extension("json.tmp")
}

/// 取出文件名（不含扩展名），用于错误信息里的标识回显。
fn file_stem_(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 组装带路径的 I/O 错误。
fn io_error_(path: &Path, source: std::io::Error) -> StoreError {
    StoreError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// 当前时间（自 Unix 纪元起的毫秒数）。
fn now_millis_() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// 把消息条数钳进 `u32`。
fn turn_count_of_(turns: &[Turn]) -> u32 {
    u32::try_from(turns.len()).unwrap_or(u32::MAX)
}

/// 决定会话标题：优先用显式标题，否则从首条用户消息推导。
fn normalize_title_(title: Option<String>, turns: &[Turn]) -> String {
    if let Some(title) = title {
        let trimmed = title.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    derive_title_(turns)
}

/// 从首条非空的用户消息推导标题。
///
/// 取第一行、去掉首尾空白、最多 [`TITLE_MAX_CHARS_`] 个字符（按**字符**截断，
/// 避免把中文截成半个）；一条都没有时用 [`DEFAULT_TITLE_`]。
fn derive_title_(turns: &[Turn]) -> String {
    for turn in turns {
        if turn.role != Role::User {
            continue;
        }
        let text = turn.text.trim();
        if text.is_empty() {
            continue;
        }
        let first_line = text.lines().next().unwrap_or(text).trim();
        let mut title: String = first_line.chars().take(TITLE_MAX_CHARS_).collect();
        if first_line.chars().count() > TITLE_MAX_CHARS_ {
            title.push('…');
        }
        if !title.is_empty() {
            return title;
        }
    }
    DEFAULT_TITLE_.to_string()
}

#[cfg(test)]
mod tests_ {
    use super::*;
    use abs_cancel::{CancelledToken, NonCancellableToken, TrMayCancel};
    use abs_kb_svc::v1::desktop::{TurnId, TurnState};

    /// 造一个临时存储根目录。
    ///
    /// 返回 `TempDir` 是为了让它在测试结束时自动清理；根目录本身再下一层，
    /// 这样 `remove_workspace` 的级联删除不会碰到测试框架的文件。
    fn temp_root_() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("应当能创建临时目录");
        let root = dir.path().join("store");
        (dir, root)
    }

    /// 造一条消息。
    fn turn_(id: &str, role: Role, text: &str) -> Turn {
        Turn {
            turn_id: TurnId::new(id),
            role,
            text: text.to_string(),
            reasoning: String::new(),
            state: TurnState::Done,
            tool_calls: Vec::new(),
            usage: None,
            notice: None,
        }
    }

    /// 测试新增的工作区会落到文档约定的路径上，且内容可读回。
    ///
    /// - 手段：在临时目录里 `Store::open`，调用 `add_workspace`，再把
    ///   `<root>/workspaces/<id>.json` 当普通 JSON 读出来。
    /// - 判断：文件存在、反序列化后与返回值完全相等——布局与内容都被钉住。
    #[compio::test]
    async fn add_workspace_writes_documented_layout_() {
        let (_guard, root) = temp_root_();
        let store = Store::open(&root).await.expect("应当能打开存储");

        let workspace = store
            .add_workspace("笔记", "/home/me/notes")
            .await
            .expect("应当能新增工作区");

        let path = root
            .join("workspaces")
            .join(format!("{}.json", workspace.workspace_id));
        assert!(path.is_file(), "文件应当存在: {}", path.display());

        let raw = std::fs::read(&path).expect("应当能读到文件");
        let parsed: Workspace = serde_json::from_slice(&raw).expect("应当是合法 JSON");
        assert_eq!(parsed, workspace);
        assert_eq!(parsed.name, "笔记");
        assert_eq!(parsed.path, "/home/me/notes");
    }

    /// 测试列表按标识升序返回全部工作区。
    ///
    /// - 手段：新增三个工作区，再 `list_workspaces`。
    /// - 判断：数量为 3，且标识序列严格递增——顺序稳定，便于人读与脚本处理。
    #[compio::test]
    async fn list_workspaces_is_sorted_by_id_() {
        let (_guard, root) = temp_root_();
        let store = Store::open(&root).await.expect("应当能打开存储");

        for name in ["甲", "乙", "丙"] {
            store
                .add_workspace(name, "/tmp/x")
                .await
                .expect("应当能新增工作区");
        }

        let list = store.list_workspaces().await.expect("应当能列出工作区");
        assert_eq!(list.workspaces.len(), 3);
        for pair in list.workspaces.windows(2) {
            assert!(
                pair[0].workspace_id < pair[1].workspace_id,
                "应当按标识升序: {:?}",
                pair
            );
        }
    }

    /// 测试"改"走的是就地覆盖，而不是新建一个对象。
    ///
    /// - 手段：新增工作区后改名字，再 `save_workspace` 写回，最后重新列出。
    /// - 判断：仍然只有一个工作区，且名字是新的——说明标识没有漂移。
    #[compio::test]
    async fn save_workspace_updates_in_place_() {
        let (_guard, root) = temp_root_();
        let store = Store::open(&root).await.expect("应当能打开存储");

        let mut workspace = store
            .add_workspace("旧名字", "/tmp/old")
            .await
            .expect("应当能新增工作区");
        let id = workspace.workspace_id.clone();

        workspace.name = "新名字".to_string();
        workspace.path = "/tmp/new".to_string();
        store
            .save_workspace(&workspace)
            .await
            .expect("应当能写回工作区");

        let list = store.list_workspaces().await.expect("应当能列出工作区");
        assert_eq!(list.workspaces.len(), 1);
        assert_eq!(list.workspaces[0].workspace_id, id);

        let read_back = store.get_workspace(&id).await.expect("应当能读回工作区");
        assert_eq!(read_back.name, "新名字");
        assert_eq!(read_back.path, "/tmp/new");
    }

    /// 测试删除工作区会级联删掉它的会话目录。
    ///
    /// - 手段：建工作区 → 建两个会话 → 删工作区，然后看会话目录是否还在。
    /// - 判断：会话目录整个消失，且被删的工作区再读会报 `NotFound`。
    #[compio::test]
    async fn remove_workspace_cascades_sessions_() {
        let (_guard, root) = temp_root_();
        let store = Store::open(&root).await.expect("应当能打开存储");

        let workspace = store
            .add_workspace("笔记", "/tmp/notes")
            .await
            .expect("应当能新增工作区");
        for title in ["甲", "乙"] {
            store
                .create_session(&workspace.workspace_id, Some(title.to_string()), Vec::new())
                .await
                .expect("应当能新建会话");
        }

        let sessions_dir = root.join("sessions").join(workspace.workspace_id.as_str());
        assert!(sessions_dir.is_dir(), "会话目录应当已建立");

        store
            .remove_workspace(&workspace.workspace_id)
            .await
            .expect("应当能删除工作区");

        assert!(!sessions_dir.exists(), "会话目录应当被级联删除");
        let error = store
            .get_workspace(&workspace.workspace_id)
            .await
            .expect_err("已经删掉的工作区不应还能读到");
        assert!(
            matches!(error, StoreError::NotFound { .. }),
            "实际错误: {error}"
        );
    }

    /// 测试新建会话会落盘全部消息，并在缺省标题时从首条用户消息推导。
    ///
    /// - 手段：建工作区，带着两条消息（第一条是用户）调 `create_session`，
    ///   `title` 传 `None`。
    /// - 判断：返回的摘要标题是首条用户消息、`turn_count` 为 2；文件落在
    ///   `sessions/<workspace_id>/<session_id>.json`，读回的 `turns` 与输入一致。
    #[compio::test]
    async fn create_session_derives_title_and_persists_turns_() {
        let (_guard, root) = temp_root_();
        let store = Store::open(&root).await.expect("应当能打开存储");
        let workspace = store
            .add_workspace("笔记", "/tmp/notes")
            .await
            .expect("应当能新增工作区");

        let turns = vec![
            turn_(
                "t-1",
                Role::User,
                "帮我把这段代码解释一下\n第二行不该进标题",
            ),
            turn_("t-2", Role::Assistant, "好的"),
        ];
        let summary = store
            .create_session(&workspace.workspace_id, None, turns.clone())
            .await
            .expect("应当能新建会话");

        assert_eq!(summary.title, "帮我把这段代码解释一下");
        assert_eq!(summary.turn_count, 2);
        assert_eq!(summary.workspace_id, workspace.workspace_id);

        let path = root
            .join("sessions")
            .join(workspace.workspace_id.as_str())
            .join(format!("{}.json", summary.session_id));
        assert!(path.is_file(), "会话文件应当存在: {}", path.display());

        let detail = store
            .get_session(&workspace.workspace_id, &summary.session_id)
            .await
            .expect("应当能读回会话");
        assert_eq!(detail.turns, turns);
        assert_eq!(detail.summary, summary);
    }

    /// 测试没有可用消息时标题回落到缺省值，显式标题优先。
    ///
    /// - 手段：分别用空消息 + `None`、空消息 + 显式标题、以及只有助手消息三种输入
    ///   新建会话。
    /// - 判断：标题依次是 `新会话`、显式标题、`新会话`——推导逻辑不会把助手消息
    ///   当成用户问题。
    #[compio::test]
    async fn session_title_falls_back_and_prefers_explicit_() {
        let (_guard, root) = temp_root_();
        let store = Store::open(&root).await.expect("应当能打开存储");
        let workspace = store
            .add_workspace("笔记", "/tmp/notes")
            .await
            .expect("应当能新增工作区");

        let empty = store
            .create_session(&workspace.workspace_id, None, Vec::new())
            .await
            .expect("应当能新建会话");
        assert_eq!(empty.title, DEFAULT_TITLE_);

        let explicit = store
            .create_session(
                &workspace.workspace_id,
                Some("自定义".to_string()),
                vec![turn_("t-1", Role::User, "会被忽略")],
            )
            .await
            .expect("应当能新建会话");
        assert_eq!(explicit.title, "自定义");

        let assistant_only = store
            .create_session(
                &workspace.workspace_id,
                Some("   ".to_string()),
                vec![turn_("t-2", Role::Assistant, "我先说话")],
            )
            .await
            .expect("应当能新建会话");
        assert_eq!(assistant_only.title, DEFAULT_TITLE_);
    }

    /// 测试会话列表只回摘要，且按最近活动时间降序。
    ///
    /// - 手段：建工作区与三个会话（逐个 `rename_session` 制造不同的活动时间），
    ///   再 `list_sessions`。
    /// - 判断：返回 3 条摘要，`updated_at_millis` 单调不增，且最后重命名的那个
    ///   排在最前——"最近用过的会话排在前面"这条界面语义。
    #[compio::test]
    async fn list_sessions_is_newest_first_() {
        let (_guard, root) = temp_root_();
        let store = Store::open(&root).await.expect("应当能打开存储");
        let workspace = store
            .add_workspace("笔记", "/tmp/notes")
            .await
            .expect("应当能新增工作区");

        let mut ids = Vec::new();
        for title in ["甲", "乙", "丙"] {
            let summary = store
                .create_session(&workspace.workspace_id, Some(title.to_string()), Vec::new())
                .await
                .expect("应当能新建会话");
            ids.push(summary.session_id);
        }

        // 制造明确的时间差：毫秒级时间戳需要一点间隔才可比。
        std::thread::sleep(std::time::Duration::from_millis(5));
        store
            .rename_session(&workspace.workspace_id, &ids[0], "甲-改")
            .await
            .expect("应当能重命名会话");

        let list = store
            .list_sessions(&workspace.workspace_id)
            .await
            .expect("应当能列出会话");
        assert_eq!(list.sessions.len(), 3);
        assert_eq!(list.sessions[0].session_id, ids[0]);
        assert_eq!(list.sessions[0].title, "甲-改");
        for pair in list.sessions.windows(2) {
            assert!(
                pair[0].updated_at_millis >= pair[1].updated_at_millis,
                "应当按活动时间降序: {:?}",
                pair
            );
        }
    }

    /// 测试追加消息会同步刷新摘要的条数与活动时间。
    ///
    /// - 手段：建一个空会话，`append_turns` 追加两条消息，再读回。
    /// - 判断：`turn_count` 为 2、`turns` 长度为 2、活动时间不小于创建时间。
    #[compio::test]
    async fn append_turns_refreshes_summary_() {
        let (_guard, root) = temp_root_();
        let store = Store::open(&root).await.expect("应当能打开存储");
        let workspace = store
            .add_workspace("笔记", "/tmp/notes")
            .await
            .expect("应当能新增工作区");
        let created = store
            .create_session(&workspace.workspace_id, None, Vec::new())
            .await
            .expect("应当能新建会话");

        let summary = store
            .append_turns(
                &workspace.workspace_id,
                &created.session_id,
                &[
                    turn_("t-1", Role::User, "你好"),
                    turn_("t-2", Role::Assistant, "你好呀"),
                ],
            )
            .await
            .expect("应当能追加消息");

        assert_eq!(summary.turn_count, 2);
        assert!(summary.updated_at_millis >= created.updated_at_millis);

        let detail = store
            .get_session(&workspace.workspace_id, &created.session_id)
            .await
            .expect("应当能读回会话");
        assert_eq!(detail.turns.len(), 2);
    }

    /// 测试删除不存在的工作区/会话会报 `NotFound`，删除存在的会真的删掉。
    ///
    /// - 手段：分别删除一个从未创建过的会话、一个从未创建过的工作区，
    ///   再创建后删除一个真实会话。
    /// - 判断：前两次是 `NotFound`；第三次成功，并且再读该会话同样报 `NotFound`。
    #[compio::test]
    async fn remove_reports_not_found_and_is_effective_() {
        let (_guard, root) = temp_root_();
        let store = Store::open(&root).await.expect("应当能打开存储");
        let workspace = store
            .add_workspace("笔记", "/tmp/notes")
            .await
            .expect("应当能新增工作区");
        let ghost_workspace = WorkspaceId::new("w-ghost");
        let ghost_session = SessionId::new("s-ghost");

        let error = store
            .remove_workspace(&ghost_workspace)
            .await
            .expect_err("不存在的工作区应当报错");
        assert!(
            matches!(error, StoreError::NotFound { .. }),
            "实际错误: {error}"
        );

        let error = store
            .remove_session(&workspace.workspace_id, &ghost_session)
            .await
            .expect_err("不存在的会话应当报错");
        assert!(
            matches!(error, StoreError::NotFound { .. }),
            "实际错误: {error}"
        );

        let summary = store
            .create_session(&workspace.workspace_id, None, Vec::new())
            .await
            .expect("应当能新建会话");
        store
            .remove_session(&workspace.workspace_id, &summary.session_id)
            .await
            .expect("应当能删除会话");

        let error = store
            .get_session(&workspace.workspace_id, &summary.session_id)
            .await
            .expect_err("已经删掉的会话不应还能读到");
        assert!(
            matches!(error, StoreError::NotFound { .. }),
            "实际错误: {error}"
        );
    }

    /// 测试会话操作要求工作区先存在。
    ///
    /// - 手段：对一个从未创建的工作区分别调用 `create_session` 与 `list_sessions`。
    /// - 判断：两者都返回 `NotFound`；尤其 `list_sessions` **不返回空列表**——
    ///   否则客户端会把"工作区标识写错"误读成"这里没有会话"。
    #[compio::test]
    async fn session_operations_require_existing_workspace_() {
        let (_guard, root) = temp_root_();
        let store = Store::open(&root).await.expect("应当能打开存储");
        let ghost = WorkspaceId::new("w-ghost");

        let error = store
            .create_session(&ghost, None, Vec::new())
            .await
            .expect_err("工作区不存在时不应能建会话");
        assert!(
            matches!(error, StoreError::NotFound { .. }),
            "实际错误: {error}"
        );

        let error = store
            .list_sessions(&ghost)
            .await
            .expect_err("工作区不存在时不应返回空列表");
        assert!(
            matches!(error, StoreError::NotFound { .. }),
            "实际错误: {error}"
        );
    }

    /// 测试含路径穿越成分的标识在碰盘之前就被拒绝。
    ///
    /// - 手段：构造 `WorkspaceId::new("../escape")` 调 `save_workspace`。
    /// - 判断：返回 `InvalidId`，且存储根目录的父目录下没有出现 `escape.json`——
    ///   "先校验、后拼路径"这条顺序真的被执行了。
    #[compio::test]
    async fn invalid_id_is_rejected_before_touching_disk_() {
        let (_guard, root) = temp_root_();
        let store = Store::open(&root).await.expect("应当能打开存储");

        let escape_target = root.parent().expect("应当有父目录").join("escape.json");
        assert!(!escape_target.exists());

        let evil = Workspace {
            workspace_id: WorkspaceId::new("../escape"),
            name: "越界".to_string(),
            path: "/tmp/evil".to_string(),
        };
        let error = store
            .save_workspace(&evil)
            .await
            .expect_err("路径穿越标识应当被拒绝");

        assert!(
            matches!(error, StoreError::InvalidId { .. }),
            "实际错误: {error}"
        );
        assert!(!escape_target.exists(), "不应在存储根目录之外留下文件");
    }

    /// 测试重新打开存储后仍能看到之前写的数据。
    ///
    /// - 手段：写一个工作区与一个会话后丢弃 `Store`，再用同一个根目录 `open` 一次。
    /// - 判断：工作区与会话都还在，会话正文也完整——持久化不依赖进程内缓存。
    #[compio::test]
    async fn reopen_sees_previous_data_() {
        let (_guard, root) = temp_root_();
        let (workspace_id, session_id) = {
            let store = Store::open(&root).await.expect("应当能打开存储");
            let workspace = store
                .add_workspace("笔记", "/tmp/notes")
                .await
                .expect("应当能新增工作区");
            let summary = store
                .create_session(
                    &workspace.workspace_id,
                    None,
                    vec![turn_("t-1", Role::User, "持久化一下")],
                )
                .await
                .expect("应当能新建会话");
            (workspace.workspace_id, summary.session_id)
        };

        let reopened = Store::open(&root).await.expect("应当能重新打开存储");
        let workspace = reopened
            .get_workspace(&workspace_id)
            .await
            .expect("应当能读回工作区");
        assert_eq!(workspace.name, "笔记");

        let detail = reopened
            .get_session(&workspace_id, &session_id)
            .await
            .expect("应当能读回会话");
        assert_eq!(detail.turns.len(), 1);
        assert_eq!(detail.turns[0].text, "持久化一下");
    }

    /// 测试被手工改坏的文件会明确报"哪个文件读不出来"。
    ///
    /// - 手段：先正常写一个工作区，再把它的 `.json` 覆盖成一串非法 JSON，
    ///   然后 `list_workspaces`。
    /// - 判断：返回 `Decode`，且错误里带着出错路径——排查时不必逐个文件试。
    #[compio::test]
    async fn broken_file_reports_decode_error_with_path_() {
        let (_guard, root) = temp_root_();
        let store = Store::open(&root).await.expect("应当能打开存储");
        let workspace = store
            .add_workspace("笔记", "/tmp/notes")
            .await
            .expect("应当能新增工作区");

        let path = root
            .join("workspaces")
            .join(format!("{}.json", workspace.workspace_id));
        std::fs::write(&path, b"{ not json").expect("应当能写坏文件");

        let error = store.list_workspaces().await.expect_err("坏文件应当报错");
        match error {
            StoreError::Decode { path: reported, .. } => assert_eq!(reported, path),
            other => panic!("应当是 Decode，实际: {other}"),
        }
    }

    /// 测试标题推导按字符（而不是字节）截断。
    ///
    /// - 手段：用一条远超上限的中文消息推导标题。
    /// - 判断：标题字符数为上限 + 1（多出的一个是省略号），且以 `…` 结尾——
    ///   不会出现半个汉字。
    #[compio::test]
    async fn derived_title_truncates_by_chars_() {
        let (_guard, root) = temp_root_();
        let store = Store::open(&root).await.expect("应当能打开存储");
        let workspace = store
            .add_workspace("笔记", "/tmp/notes")
            .await
            .expect("应当能新增工作区");

        let long = "知".repeat(TITLE_MAX_CHARS_ + 10);
        let summary = store
            .create_session(
                &workspace.workspace_id,
                None,
                vec![turn_("t-1", Role::User, &long)],
            )
            .await
            .expect("应当能新建会话");

        assert_eq!(summary.title.chars().count(), TITLE_MAX_CHARS_ + 1);
        assert!(summary.title.ends_with('…'), "实际标题: {}", summary.title);
    }

    /// 测试存储操作的可取消路径真的可用，且取消之后不留下副作用。
    ///
    /// - 手段：对一个空的存储分别用 `CancelledToken` 与 `NonCancellableToken`
    ///   走 `.may_cancel_with(token)`——前者调 `add_workspace`，后者也调一次。
    /// - 判断：已取消的令牌让操作立刻返回 [`StoreError::Cancelled`]，并且
    ///   **没有碰盘**（再列出工作区是空的）；不可取消的令牌照常完成。
    ///   这条测试守着"每个操作都由 `gen_mcf2` 展开、真的接受取消令牌"这件事。
    #[compio::test]
    async fn store_operations_honor_cancellation_() {
        let (_guard, root) = temp_root_();
        let store = Store::open(&root).await.expect("应当能打开存储");

        let cancelled = store
            .add_workspace("不该落盘", "/tmp/nope")
            .may_cancel_with(CancelledToken::new())
            .await;
        assert!(
            matches!(cancelled, Err(StoreError::Cancelled)),
            "已经取消的令牌应当让操作立刻收手，实际: {cancelled:?}"
        );
        assert!(
            store
                .list_workspaces()
                .await
                .expect("应当能列出工作区")
                .workspaces
                .is_empty(),
            "取消之后不应当留下任何工作区"
        );

        let workspace = store
            .add_workspace("笔记", "/tmp/notes")
            .may_cancel_with(NonCancellableToken::new())
            .await
            .expect("不可取消的令牌应当让操作照常完成");
        assert_eq!(workspace.name, "笔记");
    }
}
