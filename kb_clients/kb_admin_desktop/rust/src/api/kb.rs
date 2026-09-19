//! 连接 `kb_core`：读 / 写客户端自己的配置、选择连接方式、连上并列工作区 / 会话。
//!
//! # 这一层只做三件事
//!
//! 1. 把配置在 Rust 类型与**扁平视图**之间来回翻译（[`ConnectionView`] ↔
//!    `kb_client_config::Connection`）；
//! 2. 持有那个已经连上的客户端（[`connect`] 之后，[`list_workspaces`] 等直接用）；
//! 3. 把错误翻成"成功标志 + 字符串"。
//!
//! 真正的逻辑在 `kb_client_config`（配置）与 `kb_client_conn_mgr`（连接）里，
//! 这一层刻意薄——它只为"跨 FFI 好翻译"而存在。
//!
//! # 为什么视图是扁平的
//!
//! flutter_rust_bridge 会把**直接持有第三方类型字段**的结构体退化成 opaque
//! 句柄（跨 crate 的标识类型尤其如此）。所以这里的每个字段都只有
//! `String` / 整数 / `bool` / `Vec<扁平视图>`，业务类型在 Rust 侧就地翻成它们。
//!
//! # 错误怎么传
//!
//! 不用 `Result`：每个"报告"结构体带一个 `ok` 与一个 `error`，`error` 是空串
//! 表示成功。这样跨 FFI 不需要任何错误类型映射，界面判断 `ok` 即可；具体文案
//! 由 Rust 侧的 `Display` 给出（中文，可直接展示）。
//!
//! # 为什么这里是**同步**函数而不是 `async fn`
//!
//! `kb_client_conn_mgr` 的接口是异步的，但这一层刻意用 `block_on` 把它们**同步化**：
//!
//! - flutter_rust_bridge 2.13 为 `async fn` 生成的 `wrap_async` 代码在当前
//!   nightly 上会因为 rustc 的 HRTB 限制直接编译不过（"lifetime bound not
//!   satisfied"，见 rust-lang/rust#100013）。这不是本项目的代码问题，
//!   而是工具链版本组合问题；
//! - 不写 `#[frb(sync)]` 的普通函数由 FRB 放到它自己的**工作线程池**上执行，
//!   Dart 侧拿到的仍然是 `Future`，因此界面不会被阻塞；
//! - 于是"异步"这件事只发生在 Rust 内部：`block_on` 驱动的是与运行时无关的
//!   future，阻塞的等待（起进程、连 socket）本来就已经在专职线程上了。
//!
//! 等 FRB 升级到与本机 nightly 兼容的版本之后，可以把这一层改回 `async fn`；
//! 界面侧的 Dart API 形状（`Future<...>`）不会变。

use std::sync::{Arc, Mutex, MutexGuard};

use abs_kb_svc_v1_desktop::{
    AddWorkspaceRequest, AskRequest, CreateSessionRequest, LocalId, SessionDetail, SessionId,
    SessionSummary, Turn, TurnId, TurnState, Workspace, WorkspaceId,
};
use abs_llm::v1::cont::Role;
use futures_lite::future::block_on;
use kb_client_config::{config_path, ClientConfig, ConfigError, Connection, ConnectionKind};
use kb_client_conn_mgr::TrMayCancel;
use kb_client_conn_mgr::{connect, suggested_local_launch, ClientError, KbClient, TimeoutToken};

/// 已经连上的客户端 + 它是用哪条连接方式连的。
static CURRENT: Mutex<Option<(String, Arc<KbClient>)>> = Mutex::new(None);

/// 还没连上 `kb_core` 时所有操作统一的说明。
const NOT_CONNECTED_: &str = "还没有连接 kb_core";

/// 一条连接方式的扁平视图。
///
/// 三种 `kind` 共用这一个结构体（不适用的字段留空），这样界面只需要处理一种
/// 行、FRB 也只需要镜像一个类型。`kind` 的取值见 [`connection_kinds`]。
#[derive(Debug, Clone, Default)]
pub struct ConnectionView {
    /// 界面上显示的名字，也是 `default` 引用的键。
    pub name: String,

    /// 种类：`local-launch` / `local-attach` / `tcp`。
    pub kind: String,

    /// 一行人类可读的说明（界面上一眼看出"连的是什么"）。
    pub summary: String,

    /// `local-launch`：`kb_core` 可执行文件路径。
    pub kb_core: String,

    /// `local-launch` / `local-attach`：运行时目录。
    pub runtime_dir: String,

    /// `local-launch`：存储目录。
    pub storage_dir: String,

    /// `tcp`：`主机:端口`。
    pub address: String,
}

/// 配置文件的快照。
#[derive(Debug, Clone, Default)]
pub struct ConfigView {
    /// 配置文件路径（界面上显示出来，方便用户手改）。
    pub path: String,

    /// 文件是不是存在。`false` 表示**首次运行**：界面应当引导用户新建。
    pub exists: bool,

    /// 缺省用哪条连接方式的名字（可能为空）。
    pub default_name: String,

    /// 已配置的连接方式。
    pub connections: Vec<ConnectionView>,

    /// 读取 / 解析失败的说明；空串表示没问题。
    ///
    /// 注意与 `exists == false` 的区别：文件在、但内容坏了时 `exists == true`
    /// 而 `error` 非空——界面应当让用户去修，而不是当成首次运行覆盖掉。
    pub error: String,
}

/// 一次连接的尝试结果。
#[derive(Debug, Clone, Default)]
pub struct ConnectReport {
    /// 是否连上（含应用层握手成功）。
    pub ok: bool,

    /// 用的是哪条连接方式。
    pub profile_name: String,

    /// 服务端版本（握手拿到的）。
    pub server_version: String,

    /// 协议版本（握手拿到的）。
    pub protocol_version: u32,

    /// 是不是本机连接（IPC）而不是远程 TCP。
    pub is_local: bool,

    /// 客户端自己起的 `kb_core` 的 pid；附着 / 远程连接是 `0`。
    pub launched_pid: u32,

    /// 失败的说明；空串表示成功。
    pub error: String,
}

/// 当前连接的状态（给界面显示"现在连的是谁"）。
#[derive(Debug, Clone, Default)]
pub struct ConnectionState {
    /// 现在有没有连上。
    pub connected: bool,

    /// 用的是哪条连接方式。
    pub profile_name: String,

    /// 服务端版本。
    pub server_version: String,

    /// 客户端自己起的 `kb_core` 的 pid；没有则是 `0`。
    pub launched_pid: u32,
}

/// 一个工作区。
#[derive(Debug, Clone, Default)]
pub struct WorkspaceView {
    /// 工作区标识（`kb_core` 分配）。
    pub id: String,

    /// 展示名。
    pub name: String,

    /// 对应的磁盘目录。
    pub path: String,
}

/// 一个会话（只有摘要）。
#[derive(Debug, Clone, Default)]
pub struct SessionView {
    /// 会话标识。
    pub id: String,

    /// 所属工作区标识。
    pub workspace_id: String,

    /// 标题。
    pub title: String,

    /// 最近活动时间（自 Unix 纪元起的毫秒数）。
    pub updated_at_millis: i64,

    /// 该会话里已有的回合数。
    pub turn_count: u32,
}

/// 列工作区的结果。
#[derive(Debug, Clone, Default)]
pub struct WorkspacesReport {
    /// 是否成功。
    pub ok: bool,

    /// 工作区列表（失败时为空）。
    pub workspaces: Vec<WorkspaceView>,

    /// 失败的说明；空串表示成功。
    pub error: String,
}

/// 列会话的结果。
#[derive(Debug, Clone, Default)]
pub struct SessionsReport {
    /// 是否成功。
    pub ok: bool,

    /// 会话列表（失败时为空）。
    pub sessions: Vec<SessionView>,

    /// 失败的说明；空串表示成功。
    pub error: String,
}

/// 新建一个工作区的结果。
#[derive(Debug, Clone, Default)]
pub struct WorkspaceReport {
    /// 是否成功。
    pub ok: bool,

    /// 服务端建立的工作区（失败时是一个空视图）。
    pub workspace: WorkspaceView,

    /// 失败的说明；空串表示成功。
    pub error: String,
}

/// 新建一个会话的结果。
#[derive(Debug, Clone, Default)]
pub struct SessionReport {
    /// 是否成功。
    pub ok: bool,

    /// 服务端建立的会话摘要（失败时是一个空视图）。
    pub session: SessionView,

    /// 失败的说明；空串表示成功。
    pub error: String,
}

/// 一次"没有返回值"的操作（删除）的结果。
#[derive(Debug, Clone, Default)]
pub struct OpReport {
    /// 是否成功。
    pub ok: bool,

    /// 失败的说明；空串表示成功。
    pub error: String,
}

/// 会话里的一条消息（扁平视图）。
///
/// `reasoning` / `state` / `notice` 都保留了协议里的形状，即使当前的临时模拟
/// LLM 只产出"已完成的纯文本"——以后接上流式生成时，界面不用改数据形状。
#[derive(Debug, Clone, Default)]
pub struct TurnView {
    /// 回合标识。
    pub id: String,

    /// 说话人：`user` / `assistant` / `system` / `tool`。
    pub role: String,

    /// 正文。
    pub text: String,

    /// 推理正文。
    pub reasoning: String,

    /// 生成状态：`streaming` / `done` / `failed`。
    pub state: String,

    /// 提示正文；空串表示没有提示。
    pub notice: String,

    /// 提示是不是错误。
    pub notice_is_error: bool,
}

/// 一个会话的完整内容（摘要 + 全部消息）。
#[derive(Debug, Clone, Default)]
pub struct SessionDetailReport {
    /// 是否成功。
    pub ok: bool,

    /// 会话摘要（失败时是一个空视图）。
    pub session: SessionView,

    /// 会话内的全部消息，按时间顺序。
    pub turns: Vec<TurnView>,

    /// 失败的说明；空串表示成功。
    pub error: String,
}

/// 三种连接方式的 `kind` 取值（界面用它填下拉框）。
///
/// 顺序就是建议的展示顺序：先本机、后远程。
pub fn connection_kinds() -> Vec<String> {
    [
        ConnectionKind::LocalLaunch,
        ConnectionKind::LocalAttach,
        ConnectionKind::Tcp,
    ]
    .into_iter()
    .map(|kind| kind.as_str().to_string())
    .collect()
}

/// 某种连接方式的说明（界面上的提示文案）。
pub fn connection_kind_description(kind: String) -> String {
    ConnectionKind::parse(&kind)
        .map(|kind| kind.description().to_string())
        .unwrap_or_else(|| format!("未知的连接方式: {kind}"))
}

/// 配置文件的路径。
pub fn config_file_path() -> String {
    config_path()
        .map(|path| path.display().to_string())
        .unwrap_or_default()
}

/// 读配置。
///
/// 文件不存在时返回 `exists == false` 且 `error` 为空——**这不是错误**：
/// 界面据此进入"首次运行"，让用户选择或填写连接方式，再用 [`save_config`] 写出去。
pub fn load_config() -> ConfigView {
    let Some(path) = config_path() else {
        return ConfigView {
            error: ConfigError::NoConfigDir.to_string(),
            ..ConfigView::default()
        };
    };

    match ClientConfig::load(&path) {
        Ok(config) => ConfigView {
            path: path.display().to_string(),
            exists: true,
            default_name: config.default.clone().unwrap_or_default(),
            connections: config.connections.iter().map(view_of_).collect(),
            error: String::new(),
        },
        Err(ConfigError::NotFound { .. }) => ConfigView {
            path: path.display().to_string(),
            exists: false,
            error: String::new(),
            ..ConfigView::default()
        },
        Err(error) => ConfigView {
            path: path.display().to_string(),
            exists: true,
            error: error.to_string(),
            ..ConfigView::default()
        },
    }
}

/// 写配置（首次生成，或者用户改完之后保存）。
///
/// 返回空串表示成功，否则是失败说明。
pub fn save_config(default_name: String, connections: Vec<ConnectionView>) -> String {
    let Some(path) = config_path() else {
        return ConfigError::NoConfigDir.to_string();
    };

    let mut config = ClientConfig {
        version: kb_client_config::CONFIG_VERSION,
        default: (!default_name.trim().is_empty()).then(|| default_name.clone()),
        connections: connections.iter().map(connection_of_).collect(),
    };
    if config.default.is_none() {
        config.default = config
            .connections
            .first()
            .map(|connection| connection.name().to_string());
    }

    match config.save(&path) {
        Ok(()) => String::new(),
        Err(error) => error.to_string(),
    }
}

/// 首次运行时给界面一个**预填**的本机连接方式。
///
/// `kb_core` 的路径是"猜"出来的（与本可执行文件同目录），Flutter 打包出来的 App
/// 里通常猜不到——那时它是空串，界面必须让用户自己选。
pub fn suggested_local_connection(name: String) -> ConnectionView {
    view_of_(&suggested_local_launch(if name.trim().is_empty() {
        "本机".to_string()
    } else {
        name
    }))
}

/// 连上 `kb_core`（系统层 + 应用层握手）。
///
/// 成功后，[`list_workspaces`] / [`list_sessions`] 就用这条连接；再连一次会
/// 替换掉旧的（旧的被丢弃时，如果是本机启动方式，它起的 `kb_core` 也会被结束）。
pub fn connect_to(profile: ConnectionView) -> ConnectReport {
    let connection = connection_of_(&profile);
    let token = TimeoutToken::after(
        connection
            .handshake_timeout()
            .max(connection.connect_timeout()),
    );

    match block_on(connect(&connection).may_cancel_with(token)) {
        Ok(client) => {
            let report = ConnectReport {
                ok: true,
                profile_name: connection.name().to_string(),
                server_version: client.server_info().server_version.clone(),
                protocol_version: client.server_info().protocol_version,
                is_local: client.is_local(),
                launched_pid: client.launched_pid().unwrap_or(0),
                error: String::new(),
            };
            *lock_(&CURRENT) = Some((connection.name().to_string(), Arc::new(client)));
            report
        }
        Err(error) => ConnectReport {
            profile_name: connection.name().to_string(),
            error: describe_client_error_(&error),
            ..ConnectReport::default()
        },
    }
}

/// 断开当前连接（本机启动方式会把那个 `kb_core` 子进程结束掉）。
pub fn disconnect() {
    *lock_(&CURRENT) = None;
}

/// 现在连的是谁。
pub fn connection_state() -> ConnectionState {
    match current_() {
        Some((name, client)) => ConnectionState {
            connected: true,
            profile_name: name,
            server_version: client.server_info().server_version.clone(),
            launched_pid: client.launched_pid().unwrap_or(0),
        },
        None => ConnectionState::default(),
    }
}

/// 列出当前连接下的全部工作区。
pub fn list_workspaces() -> WorkspacesReport {
    let Some((_, client)) = current_() else {
        return WorkspacesReport {
            error: NOT_CONNECTED_.to_string(),
            ..WorkspacesReport::default()
        };
    };

    let token = TimeoutToken::after(client.request_timeout());
    match block_on(client.list_workspaces().may_cancel_with(token)) {
        Ok(list) => WorkspacesReport {
            ok: true,
            workspaces: list.workspaces.iter().map(workspace_view_).collect(),
            error: String::new(),
        },
        Err(error) => WorkspacesReport {
            error: describe_client_error_(&error),
            ..WorkspacesReport::default()
        },
    }
}

/// 列出某个工作区下的会话（只有摘要）。
pub fn list_sessions(workspace_id: String) -> SessionsReport {
    let Some((_, client)) = current_() else {
        return SessionsReport {
            error: NOT_CONNECTED_.to_string(),
            ..SessionsReport::default()
        };
    };

    let token = TimeoutToken::after(client.request_timeout());
    match block_on(
        client
            .list_sessions(WorkspaceId::new(workspace_id))
            .may_cancel_with(token),
    ) {
        Ok(list) => SessionsReport {
            ok: true,
            sessions: list.sessions.iter().map(session_view_).collect(),
            error: String::new(),
        },
        Err(error) => SessionsReport {
            error: describe_client_error_(&error),
            ..SessionsReport::default()
        },
    }
}

/// 在 `kb_core` 上新建一个工作区。
///
/// `path` 是 **`kb_core` 所在主机上**的目录：客户端不碰自己这边的文件系统，
/// 只把「名字 + 路径」提交给服务端，由服务端分配标识并落盘。
pub fn add_workspace(name: String, path: String) -> WorkspaceReport {
    let Some((_, client)) = current_() else {
        return WorkspaceReport {
            error: NOT_CONNECTED_.to_string(),
            ..WorkspaceReport::default()
        };
    };

    let token = TimeoutToken::after(client.request_timeout());
    let request = AddWorkspaceRequest {
        // `local_id` 只是线上往返一次的簿记字段：服务端分配完标识后，
        // 客户端这边用不上它（真正要的是应答里的 `workspace_id`）。
        local_id: LocalId::generate(),
        name,
        path,
    };

    match block_on(client.add_workspace(request).may_cancel_with(token)) {
        Ok(workspace) => WorkspaceReport {
            ok: true,
            workspace: workspace_view_(&workspace),
            error: String::new(),
        },
        Err(error) => WorkspaceReport {
            error: describe_client_error_(&error),
            ..WorkspaceReport::default()
        },
    }
}

/// 删除一个工作区；`kb_core` 会**级联删除**它名下的会话。
pub fn remove_workspace(workspace_id: String) -> OpReport {
    let Some((_, client)) = current_() else {
        return OpReport {
            error: NOT_CONNECTED_.to_string(),
            ..OpReport::default()
        };
    };

    let token = TimeoutToken::after(client.request_timeout());
    match block_on(
        client
            .remove_workspace(WorkspaceId::new(workspace_id))
            .may_cancel_with(token),
    ) {
        Ok(()) => OpReport {
            ok: true,
            error: String::new(),
        },
        Err(error) => OpReport {
            error: describe_client_error_(&error),
            ..OpReport::default()
        },
    }
}

/// 在某个工作区下新建一个会话（可选地带上**第一个问题**）。
///
/// 两种典型用法：
///
/// - `turn_id` 为空串：建一个空会话。此时 `title` **必须**非空——`kb_core` 不
///   允许"没有消息、又只有默认名字"的会话落盘；
/// - `turn_id` 非空：把 `question` 作为会话的第一条用户消息一起提交。`kb_core`
///   会据此给会话起名（问题开头若干字），这正是客户端"草稿会话首次提问"的用法：
///   紧接着再调 [`ask`]，服务端会按 `turn_id` 去重，不会重复添加这条消息。
pub fn create_session(
    workspace_id: String,
    title: String,
    turn_id: String,
    question: String,
) -> SessionReport {
    let Some((_, client)) = current_() else {
        return SessionReport {
            error: NOT_CONNECTED_.to_string(),
            ..SessionReport::default()
        };
    };

    let trimmed = title.trim();
    let turns = if turn_id.trim().is_empty() {
        Vec::new()
    } else {
        vec![Turn {
            turn_id: TurnId::new(turn_id),
            role: Role::User,
            text: question,
            reasoning: String::new(),
            state: TurnState::Done,
            tool_calls: Vec::new(),
            usage: None,
            notice: None,
        }]
    };

    let request = CreateSessionRequest {
        workspace_id: WorkspaceId::new(workspace_id),
        local_id: LocalId::generate(),
        title: (!trimmed.is_empty()).then(|| trimmed.to_string()),
        turns,
    };

    let token = TimeoutToken::after(client.request_timeout());
    match block_on(client.create_session(request).may_cancel_with(token)) {
        Ok(session) => SessionReport {
            ok: true,
            session: session_view_(&session),
            error: String::new(),
        },
        Err(error) => SessionReport {
            error: describe_client_error_(&error),
            ..SessionReport::default()
        },
    }
}

/// 重命名一个工作区（只改展示名，磁盘目录不动）。
pub fn rename_workspace(workspace_id: String, name: String) -> WorkspaceReport {
    let Some((_, client)) = current_() else {
        return WorkspaceReport {
            error: NOT_CONNECTED_.to_string(),
            ..WorkspaceReport::default()
        };
    };

    let token = TimeoutToken::after(client.request_timeout());
    match block_on(
        client
            .rename_workspace(WorkspaceId::new(workspace_id), name)
            .may_cancel_with(token),
    ) {
        Ok(workspace) => WorkspaceReport {
            ok: true,
            workspace: workspace_view_(&workspace),
            error: String::new(),
        },
        Err(error) => WorkspaceReport {
            error: describe_client_error_(&error),
            ..WorkspaceReport::default()
        },
    }
}

/// 重命名一个会话（改标题）；`title` 只有空白时由服务端重新推导。
pub fn rename_session(
    workspace_id: String,
    session_id: String,
    title: String,
) -> SessionReport {
    let Some((_, client)) = current_() else {
        return SessionReport {
            error: NOT_CONNECTED_.to_string(),
            ..SessionReport::default()
        };
    };

    let token = TimeoutToken::after(client.request_timeout());
    match block_on(
        client
            .rename_session(
                WorkspaceId::new(workspace_id),
                SessionId::new(session_id),
                title,
            )
            .may_cancel_with(token),
    ) {
        Ok(session) => SessionReport {
            ok: true,
            session: session_view_(&session),
            error: String::new(),
        },
        Err(error) => SessionReport {
            error: describe_client_error_(&error),
            ..SessionReport::default()
        },
    }
}

/// 删除一个会话。
pub fn remove_session(workspace_id: String, session_id: String) -> OpReport {
    let Some((_, client)) = current_() else {
        return OpReport {
            error: NOT_CONNECTED_.to_string(),
            ..OpReport::default()
        };
    };

    let token = TimeoutToken::after(client.request_timeout());
    match block_on(
        client
            .remove_session(WorkspaceId::new(workspace_id), SessionId::new(session_id))
            .may_cancel_with(token),
    ) {
        Ok(()) => OpReport {
            ok: true,
            error: String::new(),
        },
        Err(error) => OpReport {
            error: describe_client_error_(&error),
            ..OpReport::default()
        },
    }
}

/// 读取一个会话的完整内容（摘要 + 全部消息）。
pub fn get_session(workspace_id: String, session_id: String) -> SessionDetailReport {
    let Some((_, client)) = current_() else {
        return SessionDetailReport {
            error: NOT_CONNECTED_.to_string(),
            ..SessionDetailReport::default()
        };
    };

    let token = TimeoutToken::after(client.request_timeout());
    match block_on(
        client
            .get_session(WorkspaceId::new(workspace_id), SessionId::new(session_id))
            .may_cancel_with(token),
    ) {
        Ok(detail) => detail_report_(detail),
        Err(error) => SessionDetailReport {
            error: describe_client_error_(&error),
            ..SessionDetailReport::default()
        },
    }
}

/// 就某个会话提问，回来后拿到**提问之后**的会话内容。
///
/// `kb_core` 现在跑的是**临时模拟的 LLM**：它把问题逆序输出当作回答，并把
/// 一问一答一起落盘。所以调用方不需要再调 [`get_session`]——返回值里已经有
/// 刚产生的两条消息。等流式生成落地后，这个入口会改成订阅事件。
///
/// `turn_id` 由调用方生成：这样"发出提问"到"拿到回答"之间界面也有标识可用。
pub fn ask(
    workspace_id: String,
    session_id: String,
    turn_id: String,
    question: String,
) -> SessionDetailReport {
    let Some((_, client)) = current_() else {
        return SessionDetailReport {
            error: NOT_CONNECTED_.to_string(),
            ..SessionDetailReport::default()
        };
    };

    let token = TimeoutToken::after(client.request_timeout());
    let request = AskRequest {
        workspace_id: WorkspaceId::new(workspace_id),
        session_id: SessionId::new(session_id),
        turn_id: TurnId::new(turn_id),
        question,
        // `None` = 用当前生效的服务；配置域还没接，模拟 LLM 不看它。
        service_id: None,
    };

    match block_on(client.ask(request).may_cancel_with(token)) {
        Ok(detail) => detail_report_(detail),
        Err(error) => SessionDetailReport {
            error: describe_client_error_(&error),
            ..SessionDetailReport::default()
        },
    }
}

/// 把协议里的工作区翻成扁平视图。
fn workspace_view_(workspace: &Workspace) -> WorkspaceView {
    WorkspaceView {
        id: workspace.workspace_id.to_string(),
        name: workspace.name.clone(),
        path: workspace.path.clone(),
    }
}

/// 把协议里的会话摘要翻成扁平视图。
fn session_view_(session: &SessionSummary) -> SessionView {
    SessionView {
        id: session.session_id.to_string(),
        workspace_id: session.workspace_id.to_string(),
        title: session.title.clone(),
        updated_at_millis: session.updated_at_millis,
        turn_count: session.turn_count,
    }
}

/// 把协议里的一条消息翻成扁平视图。
fn turn_view_(turn: &Turn) -> TurnView {
    TurnView {
        id: turn.turn_id.to_string(),
        role: role_name_(&turn.role).to_string(),
        text: turn.text.clone(),
        reasoning: turn.reasoning.clone(),
        state: state_name_(turn.state).to_string(),
        notice: turn
            .notice
            .as_ref()
            .map(|notice| notice.message.clone())
            .unwrap_or_default(),
        notice_is_error: turn
            .notice
            .as_ref()
            .is_some_and(|notice| notice.is_error),
    }
}

/// 把协议里的会话详情翻成扁平报告。
fn detail_report_(detail: SessionDetail) -> SessionDetailReport {
    SessionDetailReport {
        ok: true,
        session: session_view_(&detail.summary),
        turns: detail.turns.iter().map(turn_view_).collect(),
        error: String::new(),
    }
}

/// 说话人 → 界面用的短名（`abs_llm::v1::cont::Role` 的四个取值）。
fn role_name_(role: &Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

/// 生成状态 → 界面用的短名。
fn state_name_(state: TurnState) -> &'static str {
    match state {
        TurnState::Streaming => "streaming",
        TurnState::Done => "done",
        TurnState::Failed => "failed",
    }
}

/// 把一条连接方式翻成扁平视图。
fn view_of_(connection: &Connection) -> ConnectionView {
    let mut view = ConnectionView {
        name: connection.name().to_string(),
        kind: connection.kind().as_str().to_string(),
        summary: connection_summary_(connection),
        ..ConnectionView::default()
    };

    match connection {
        Connection::LocalLaunch {
            kb_core,
            runtime_dir,
            storage_dir,
            ..
        } => {
            view.kb_core = kb_core.display().to_string();
            view.runtime_dir = runtime_dir.display().to_string();
            view.storage_dir = storage_dir.display().to_string();
        }
        Connection::LocalAttach { runtime_dir, .. } => {
            view.runtime_dir = runtime_dir.display().to_string();
        }
        Connection::Tcp { address, .. } => {
            view.address = address.clone();
        }
    }

    view
}

/// 把扁平视图翻回连接方式。
fn connection_of_(view: &ConnectionView) -> Connection {
    match ConnectionKind::parse(&view.kind) {
        Some(ConnectionKind::LocalLaunch) => Connection::local_launch(
            view.name.clone(),
            view.kb_core.clone(),
            view.runtime_dir.clone(),
            view.storage_dir.clone(),
        ),
        Some(ConnectionKind::LocalAttach) => {
            Connection::local_attach(view.name.clone(), view.runtime_dir.clone())
        }
        Some(ConnectionKind::Tcp) => Connection::tcp(view.name.clone(), view.address.clone()),
        // 认不出的 `kind` 当成"附着到本机"，让后面的校验/连接给出明确错误，
        // 而不是在这里 panic。
        None => Connection::local_attach(view.name.clone(), view.runtime_dir.clone()),
    }
}

/// 界面上的一行说明。
fn connection_summary_(connection: &Connection) -> String {
    match connection {
        Connection::LocalLaunch {
            kb_core,
            runtime_dir,
            ..
        } => format!(
            "启动 {} 并连 {}",
            if kb_core.as_os_str().is_empty() {
                "（未指定 kb-core）"
            } else {
                "本机 kb-core"
            },
            runtime_dir.display()
        ),
        Connection::LocalAttach { runtime_dir, .. } => {
            format!("连接已在跑的 kb-core（{}）", runtime_dir.display())
        }
        Connection::Tcp { address, .. } => format!("经 kb_core_rproxy 连 {address}"),
    }
}

/// 错误文案：超时/取消与真正的失败分开说。
fn describe_client_error_(error: &ClientError) -> String {
    if error.is_cancelled() {
        "连接超时（配置里的时限到了）".to_string()
    } else {
        error.to_string()
    }
}

/// 取出当前客户端（克隆一个 `Arc`，不把锁带到 await 里）。
fn current_() -> Option<(String, Arc<KbClient>)> {
    lock_(&CURRENT).clone()
}

/// 取锁，忽略"中毒"：临界区里不会 panic。
fn lock_<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
