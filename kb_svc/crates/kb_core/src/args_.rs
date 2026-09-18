//! 命令行参数与子命令的解析。
//!
//! 本阶段 `kb-core` 有两种运行方式：
//!
//! - **不带子命令**：常驻骨架（见 [`crate::serve_`]），准备目录与存储后等 `Ctrl-C`；
//! - **带子命令**：执行一次工作区/会话操作后退出（见 [`crate::cli_`]），
//!   用于在 IPC 接通之前手工检验存储层。
//!
//! 解析刻意写得很朴素：全局选项只认 `--runtime-dir` / `--storage-dir`，
//! 之后第一个非选项 token 进入子命令；子命令内部按「`--键 值` + 位置参数」解析，
//! **顺序不敏感**。不支持 `--key=value`、不合并短选项——用不到的形式不写。

use std::path::PathBuf;

use thiserror::Error;

/// 顶层用法说明。
///
/// 直接由 `--help` 打印；README 的第一屏也以它为准。
pub const USAGE: &str = "\
kb-core —— 知识库主进程

用法:
    kb-core [--runtime-dir <目录>] [--storage-dir <目录>]
        常驻骨架：准备目录与存储、打印现状，然后等 Ctrl-C。

    kb-core [--runtime-dir <目录>] [--storage-dir <目录>] <子命令>
        执行一次操作后退出。

    kb-core --help
        打印本说明。

全局选项:
    --runtime-dir <目录>  运行时目录（下一轮的 IPC 端点放在这里）。
                          默认 $XDG_RUNTIME_DIR/llm_kb，未设置时回退系统临时目录。
    --storage-dir <目录>  工作区与会话的存储目录。默认 <运行时目录>/storage。
    --handshake-prompt <方式>
                          启动时如何把 IPC 端点文件名通知给父进程。
                          none（默认）：不通知，stdout 保持干净。
                          stdio：往 stdout 打一行 JSON 通知，供启动它的父进程读取。
    -h, --help            打印本说明。

子命令:
    workspace add    --name <名字> --path <目录>
    workspace list
    workspace show   <工作区标识>
    workspace update <工作区标识> [--name <名字>] [--path <目录>]
    workspace remove <工作区标识>

    session add    --workspace <工作区标识> [--title <标题>]
    session list   --workspace <工作区标识>
    session show   --workspace <工作区标识> <会话标识>
    session update --workspace <工作区标识> <会话标识> --title <标题>
    session remove --workspace <工作区标识> <会话标识>
";

/// 解析后的命令行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// 运行时目录与存储目录。
    pub paths: Paths,

    /// 启动时如何把 IPC 端点文件名通知给父进程。
    pub handshake_prompt: HandshakePrompt,

    /// 要执行的动作。
    pub command: Command,
}

/// 启动时把 IPC 端点文件名通知给父进程的方式。
///
/// 这属于**系统层握手**（"父进程怎么知道连哪里"），与应用层握手是两回事。
/// 通知的**消息类型**是公开协议的一部分
/// （`abs_kb_core_handshake::IpcReadyNotice`），本选项只决定**打不打**它。
/// 分工见 `abs_kb_svc::v1::desktop::handshake_` 的模块文档。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HandshakePrompt {
    /// 不通知（默认）：stdout 一个字都不多。
    #[default]
    None,

    /// 往 stdout 打一行机器可读的通知（见 `serve_::run`），供父进程读取。
    Stdio,
}

impl HandshakePrompt {
    /// 从命令行取值解析。
    fn parse_(value: &str) -> Result<Self, UsageError> {
        match value {
            "none" => Ok(Self::None),
            "stdio" => Ok(Self::Stdio),
            other => Err(UsageError::InvalidOptionValue {
                option: "handshake-prompt",
                value: other.to_string(),
            }),
        }
    }
}

/// 运行时目录与存储目录。
///
/// 两者分开：运行时目录放"进程活着才有意义"的东西（下一轮的 IPC 端点、
/// 锁文件），存储目录放工作区与会话。默认前者包含后者，但可以用
/// `--storage-dir` 指到别处。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// 运行时文件目录。
    pub runtime_dir: PathBuf,

    /// 工作区与会话的存储目录。
    pub storage_dir: PathBuf,
}

/// 顶层动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// 打印用法说明。
    Help,

    /// 常驻骨架。
    Serve,

    /// 工作区操作。
    Workspace(WorkspaceCommand),

    /// 会话操作。
    Session(SessionCommand),
}

/// 工作区的增删查改。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceCommand {
    /// 新建工作区。
    Add {
        /// 展示名。
        name: String,

        /// 对应的磁盘目录。
        path: String,
    },

    /// 列出全部工作区。
    List,

    /// 查看单个工作区。
    Show {
        /// 目标工作区标识。
        workspace_id: String,
    },

    /// 修改工作区的名字或路径；两者至少要给一个。
    Update {
        /// 目标工作区标识。
        workspace_id: String,

        /// 新的展示名；`None` 表示不改。
        name: Option<String>,

        /// 新的磁盘目录；`None` 表示不改。
        path: Option<String>,
    },

    /// 删除工作区（连同它的会话）。
    Remove {
        /// 目标工作区标识。
        workspace_id: String,
    },
}

/// 会话的增删查改。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionCommand {
    /// 新建会话。
    Add {
        /// 所属工作区标识。
        workspace_id: String,

        /// 标题；`None` 表示由服务端从消息内容推导。
        title: Option<String>,
    },

    /// 列出某个工作区下的会话。
    List {
        /// 所属工作区标识。
        workspace_id: String,
    },

    /// 查看一个会话的完整内容。
    Show {
        /// 所属工作区标识。
        workspace_id: String,

        /// 目标会话标识。
        session_id: String,
    },

    /// 修改会话标题。
    Update {
        /// 所属工作区标识。
        workspace_id: String,

        /// 目标会话标识。
        session_id: String,

        /// 新标题。
        title: String,
    },

    /// 删除会话。
    Remove {
        /// 所属工作区标识。
        workspace_id: String,

        /// 目标会话标识。
        session_id: String,
    },
}

/// 命令行解析失败。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum UsageError {
    /// 出现了不认识的选项。
    #[error("未知选项: {0}（用 --help 查看用法）")]
    UnknownOption(String),

    /// 某个选项的取值不合法。
    #[error("选项 --{option} 的取值不合法: {value:?}（可用值见 --help）")]
    InvalidOptionValue {
        /// 选项名（不含前导 `--`）。
        option: &'static str,

        /// 被拒绝的取值。
        value: String,
    },

    /// 选项后面没有取值。
    #[error("选项 {0} 缺少取值")]
    MissingValue(String),

    /// 必需的位置参数没给。
    #[error("缺少参数: {0}")]
    MissingOperand(&'static str),

    /// 必需的关键字参数没给。
    #[error("缺少必需选项: --{0}")]
    MissingOption(&'static str),

    /// 多给了位置参数。
    #[error("多余的参数: {0}")]
    UnexpectedArgument(String),

    /// 不认识的子命令（第一级）。
    #[error("未知子命令: {0}（可用: workspace / session）")]
    UnknownCommand(String),

    /// 不认识的子命令动作（第二级）。
    #[error("未知动作: {0} {1}")]
    UnknownAction(String, String),

    /// 取值组合不成立（例如 `workspace update` 一个字段都没给）。
    #[error("{0}")]
    Invalid(&'static str),
}

/// 解析命令行。
///
/// `argv` 不含程序名本身（调用方传 `std::env::args().skip(1)`）。
///
/// # Errors
///
/// 未知选项、缺取值、缺参数、未知子命令等一律返回 [`UsageError`]；
/// 调用方应当把它打到 stderr 并以退出码 2 结束。
///
/// # Examples
///
/// 本 crate 是**可执行程序**（没有 lib target），因此文档测试无法 `use kb_core`；
/// 这里用文字给出等价示例，可运行的验证见本文件的单元测试：
///
/// ```text
/// parse(["--storage-dir", "/tmp/kb", "workspace", "list"])
///   => Parsed { paths.storage_dir: "/tmp/kb",
///               command: Command::Workspace(WorkspaceCommand::List) }
///
/// parse(["--runtime-dir", "/run/kb"])
///   => Parsed { paths.runtime_dir: "/run/kb",
///               paths.storage_dir: "/run/kb/storage",
///               command: Command::Serve }
/// ```
pub fn parse<I>(argv: I) -> Result<Parsed, UsageError>
where
    I: IntoIterator<Item = String>,
{
    let tokens: Vec<String> = argv.into_iter().collect();
    let mut index = 0;
    let mut runtime_dir = None;
    let mut storage_dir = None;
    let mut handshake_prompt = HandshakePrompt::None;

    while index < tokens.len() {
        match tokens[index].as_str() {
            "--runtime-dir" => {
                runtime_dir = Some(PathBuf::from(value_at_(
                    &tokens,
                    index + 1,
                    "--runtime-dir",
                )?));
                index += 2;
            }
            "--storage-dir" => {
                storage_dir = Some(PathBuf::from(value_at_(
                    &tokens,
                    index + 1,
                    "--storage-dir",
                )?));
                index += 2;
            }
            "--handshake-prompt" => {
                let value = value_at_(&tokens, index + 1, "--handshake-prompt")?;
                handshake_prompt = HandshakePrompt::parse_(&value)?;
                index += 2;
            }
            "-h" | "--help" => {
                return Ok(Parsed {
                    paths: resolve_paths_(runtime_dir, storage_dir, xdg_runtime_dir_()),
                    handshake_prompt,
                    command: Command::Help,
                });
            }
            other if other.starts_with('-') => {
                return Err(UsageError::UnknownOption(other.to_string()));
            }
            // 第一个非选项 token 起进入子命令；不再回头解析全局选项。
            _ => break,
        }
    }

    let paths = resolve_paths_(runtime_dir, storage_dir, xdg_runtime_dir_());
    let command = if index >= tokens.len() {
        Command::Serve
    } else {
        parse_command_(&tokens[index..])?
    };

    Ok(Parsed {
        paths,
        handshake_prompt,
        command,
    })
}

/// 解析子命令部分（`tokens[0]` 是子命令名）。
fn parse_command_(tokens: &[String]) -> Result<Command, UsageError> {
    let domain = tokens[0].as_str();
    let action = tokens.get(1).map(String::as_str);

    match (domain, action) {
        ("workspace", Some("add")) => {
            let mut flags = Flags_::parse_(&tokens[2..])?;
            let name = flags.require_("name")?;
            let path = flags.require_("path")?;
            flags.finish_()?;
            Ok(Command::Workspace(WorkspaceCommand::Add { name, path }))
        }
        ("workspace", Some("list")) => {
            Flags_::parse_(&tokens[2..])?.finish_()?;
            Ok(Command::Workspace(WorkspaceCommand::List))
        }
        ("workspace", Some("show")) => {
            let mut flags = Flags_::parse_(&tokens[2..])?;
            let workspace_id = flags.operand_("<工作区标识>")?;
            flags.finish_()?;
            Ok(Command::Workspace(WorkspaceCommand::Show { workspace_id }))
        }
        ("workspace", Some("update")) => {
            let mut flags = Flags_::parse_(&tokens[2..])?;
            let workspace_id = flags.operand_("<工作区标识>")?;
            let name = flags.take_("name");
            let path = flags.take_("path");
            flags.finish_()?;
            if name.is_none() && path.is_none() {
                return Err(UsageError::Invalid(
                    "workspace update 至少要给出 --name 或 --path 之一",
                ));
            }
            Ok(Command::Workspace(WorkspaceCommand::Update {
                workspace_id,
                name,
                path,
            }))
        }
        ("workspace", Some("remove")) => {
            let mut flags = Flags_::parse_(&tokens[2..])?;
            let workspace_id = flags.operand_("<工作区标识>")?;
            flags.finish_()?;
            Ok(Command::Workspace(WorkspaceCommand::Remove {
                workspace_id,
            }))
        }
        ("workspace", Some(other)) => Err(UsageError::UnknownAction(
            "workspace".to_string(),
            other.to_string(),
        )),
        ("workspace", None) => Err(UsageError::MissingOperand("workspace <动作>")),

        ("session", Some("add")) => {
            let mut flags = Flags_::parse_(&tokens[2..])?;
            let workspace_id = flags.require_("workspace")?;
            let title = flags.take_("title");
            flags.finish_()?;
            Ok(Command::Session(SessionCommand::Add {
                workspace_id,
                title,
            }))
        }
        ("session", Some("list")) => {
            let mut flags = Flags_::parse_(&tokens[2..])?;
            let workspace_id = flags.require_("workspace")?;
            flags.finish_()?;
            Ok(Command::Session(SessionCommand::List { workspace_id }))
        }
        ("session", Some("show")) => {
            let mut flags = Flags_::parse_(&tokens[2..])?;
            let workspace_id = flags.require_("workspace")?;
            let session_id = flags.operand_("<会话标识>")?;
            flags.finish_()?;
            Ok(Command::Session(SessionCommand::Show {
                workspace_id,
                session_id,
            }))
        }
        ("session", Some("update")) => {
            let mut flags = Flags_::parse_(&tokens[2..])?;
            let workspace_id = flags.require_("workspace")?;
            let session_id = flags.operand_("<会话标识>")?;
            let title = flags.require_("title")?;
            flags.finish_()?;
            Ok(Command::Session(SessionCommand::Update {
                workspace_id,
                session_id,
                title,
            }))
        }
        ("session", Some("remove")) => {
            let mut flags = Flags_::parse_(&tokens[2..])?;
            let workspace_id = flags.require_("workspace")?;
            let session_id = flags.operand_("<会话标识>")?;
            flags.finish_()?;
            Ok(Command::Session(SessionCommand::Remove {
                workspace_id,
                session_id,
            }))
        }
        ("session", Some(other)) => Err(UsageError::UnknownAction(
            "session".to_string(),
            other.to_string(),
        )),
        ("session", None) => Err(UsageError::MissingOperand("session <动作>")),

        (other, _) => Err(UsageError::UnknownCommand(other.to_string())),
    }
}

/// 决定运行时目录与存储目录。
///
/// 优先级：
///
/// - 运行时目录：显式 `--runtime-dir` > `$XDG_RUNTIME_DIR/llm_kb` > 系统临时目录下的 `llm_kb`；
/// - 存储目录：显式 `--storage-dir` > `<运行时目录>/storage`。
///
/// 把 `xdg_runtime_dir` 作为参数而不是在函数里读环境变量，是为了让这段逻辑
/// 可以被确定性地测试。
fn resolve_paths_(
    runtime_dir: Option<PathBuf>,
    storage_dir: Option<PathBuf>,
    xdg_runtime_dir: Option<PathBuf>,
) -> Paths {
    let runtime_dir = match runtime_dir {
        // 显式给出的目录原样使用，不再拼一层 `llm_kb`。
        Some(dir) => dir,
        // `$XDG_RUNTIME_DIR` 是所有应用共用的目录，因此要加一层应用专属子目录；
        // 系统临时目录同理。
        None => xdg_runtime_dir
            .unwrap_or_else(std::env::temp_dir)
            .join("llm_kb"),
    };
    let storage_dir = storage_dir.unwrap_or_else(|| runtime_dir.join("storage"));

    Paths {
        runtime_dir,
        storage_dir,
    }
}

/// 读取 `$XDG_RUNTIME_DIR`（空值视作未设置）。
fn xdg_runtime_dir_() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// 取 `tokens[index]` 作为 `flag` 的取值。
fn value_at_(tokens: &[String], index: usize, flag: &str) -> Result<String, UsageError> {
    tokens
        .get(index)
        .cloned()
        .ok_or_else(|| UsageError::MissingValue(flag.to_string()))
}

/// 一组「`--键 值` + 位置参数」。
///
/// 关键点是**顺序不敏感**：`--name X --path Y` 与 `--path Y --name X` 等价，
/// 位置参数则按出现顺序排。
struct Flags_ {
    /// 已识别的 `--键 值`，键不含前导 `--`。
    values_: Vec<(String, String)>,

    /// 非选项参数，按出现顺序。
    operands_: Vec<String>,
}

impl Flags_ {
    /// 解析一组 token。
    fn parse_(tokens: &[String]) -> Result<Self, UsageError> {
        let mut values = Vec::new();
        let mut operands = Vec::new();
        let mut index = 0;

        while index < tokens.len() {
            let token = tokens[index].as_str();
            if let Some(name) = token.strip_prefix("--") {
                let value = value_at_(tokens, index + 1, token)?;
                values.push((name.to_string(), value));
                index += 2;
            } else if token.starts_with('-') && token != "-" {
                return Err(UsageError::UnknownOption(token.to_string()));
            } else {
                operands.push(token.to_string());
                index += 1;
            }
        }

        Ok(Self {
            values_: values,
            operands_: operands,
        })
    }

    /// 取出一个可选关键字参数。
    fn take_(&mut self, name: &str) -> Option<String> {
        let position = self.values_.iter().position(|(key, _)| key == name)?;
        Some(self.values_.remove(position).1)
    }

    /// 取出一个必需关键字参数。
    fn require_(&mut self, name: &'static str) -> Result<String, UsageError> {
        self.take_(name).ok_or(UsageError::MissingOption(name))
    }

    /// 取出第一个位置参数（本阶段的每个子命令至多有一个）。
    fn operand_(&mut self, what: &'static str) -> Result<String, UsageError> {
        if self.operands_.is_empty() {
            Err(UsageError::MissingOperand(what))
        } else {
            Ok(self.operands_.remove(0))
        }
    }

    /// 收尾：不许留下没被消费的关键字参数或多余的位置参数。
    fn finish_(&self) -> Result<(), UsageError> {
        if let Some((name, _)) = self.values_.first() {
            return Err(UsageError::UnknownOption(format!("--{name}")));
        }
        if let Some(extra) = self.operands_.first() {
            return Err(UsageError::UnexpectedArgument(extra.clone()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests_ {
    use super::*;

    /// 把 `&str` 数组转成 `parse` 要的 `String` 迭代器。
    fn argv_(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    /// 测试不带子命令时进入常驻骨架，且默认路径可以推导出来。
    ///
    /// - 手段：只给显式的 `--runtime-dir`，不带任何子命令。
    /// - 判断：`command` 是 `Serve`；存储目录默认落在运行时目录之下。
    #[test]
    fn parse_without_subcommand_is_serve_() {
        let parsed = parse(argv_(&["--runtime-dir", "/run/kb"])).expect("应当能解析");

        assert_eq!(parsed.command, Command::Serve);
        assert_eq!(parsed.paths.runtime_dir, PathBuf::from("/run/kb"));
        assert_eq!(parsed.paths.storage_dir, PathBuf::from("/run/kb/storage"));
    }

    /// 测试 `--storage-dir` 能把数据指到运行时目录之外。
    ///
    /// - 手段：同时给出两个目录。
    /// - 判断：两者各自生效，存储目录**不**在运行时目录之下。
    #[test]
    fn explicit_storage_dir_is_independent_() {
        let parsed = parse(argv_(&[
            "--storage-dir",
            "/data/kb",
            "--runtime-dir",
            "/run/kb",
        ]))
        .expect("应当能解析");

        assert_eq!(parsed.paths.runtime_dir, PathBuf::from("/run/kb"));
        assert_eq!(parsed.paths.storage_dir, PathBuf::from("/data/kb"));
    }

    /// 测试默认路径的推导规则（显式 > XDG > 系统临时目录）。
    ///
    /// - 手段：分别用「显式运行时目录 + XDG」「只有 XDG」「什么都没有」调用
    ///   纯函数 `resolve_paths_`。
    /// - 判断：显式目录压过 XDG；只有 XDG 时用 `$XDG_RUNTIME_DIR/llm_kb`；
    ///   都没有时退到系统临时目录，且存储目录始终默认在运行时目录之下。
    #[test]
    fn path_resolution_prefers_explicit_then_xdg_() {
        let explicit = resolve_paths_(
            Some(PathBuf::from("/run/kb")),
            None,
            Some(PathBuf::from("/run/user/1000")),
        );
        assert_eq!(explicit.runtime_dir, PathBuf::from("/run/kb"));

        let xdg = resolve_paths_(None, None, Some(PathBuf::from("/run/user/1000")));
        assert_eq!(xdg.runtime_dir, PathBuf::from("/run/user/1000/llm_kb"));
        assert_eq!(
            xdg.storage_dir,
            PathBuf::from("/run/user/1000/llm_kb/storage")
        );

        let fallback = resolve_paths_(None, None, None);
        assert_eq!(fallback.runtime_dir, std::env::temp_dir().join("llm_kb"));
        assert_eq!(fallback.storage_dir, fallback.runtime_dir.join("storage"));
    }

    /// 测试 `--help` 优先于其它解析结果。
    ///
    /// - 手段：在 `--help` 前后各放一个全局选项。
    /// - 判断：返回 `Command::Help`，而不是继续去解析后面的子命令。
    #[test]
    fn parse_help_wins_() {
        for argv in [
            argv_(&["--help"]),
            argv_(&["--runtime-dir", "/run/kb", "-h", "workspace", "list"]),
        ] {
            let parsed = parse(argv).expect("应当能解析");
            assert_eq!(parsed.command, Command::Help);
        }
    }

    /// 测试工作区子命令的解析结果。
    ///
    /// - 手段：逐个解析 `add` / `list` / `show` / `update` / `remove`。
    /// - 判断：每个都得到预期的枚举值；`add` 的选项顺序颠倒也能解析出同样结果。
    #[test]
    fn parse_workspace_commands_() {
        let add = parse(argv_(&[
            "workspace",
            "add",
            "--path",
            "/tmp/notes",
            "--name",
            "笔记",
        ]))
        .expect("应当能解析");
        assert_eq!(
            add.command,
            Command::Workspace(WorkspaceCommand::Add {
                name: "笔记".to_string(),
                path: "/tmp/notes".to_string(),
            })
        );

        let list = parse(argv_(&["workspace", "list"])).expect("应当能解析");
        assert_eq!(list.command, Command::Workspace(WorkspaceCommand::List));

        let show = parse(argv_(&["workspace", "show", "w-1"])).expect("应当能解析");
        assert_eq!(
            show.command,
            Command::Workspace(WorkspaceCommand::Show {
                workspace_id: "w-1".to_string(),
            })
        );

        let update =
            parse(argv_(&["workspace", "update", "w-1", "--name", "新名字"])).expect("应当能解析");
        assert_eq!(
            update.command,
            Command::Workspace(WorkspaceCommand::Update {
                workspace_id: "w-1".to_string(),
                name: Some("新名字".to_string()),
                path: None,
            })
        );

        let remove = parse(argv_(&["workspace", "remove", "w-1"])).expect("应当能解析");
        assert_eq!(
            remove.command,
            Command::Workspace(WorkspaceCommand::Remove {
                workspace_id: "w-1".to_string(),
            })
        );
    }

    /// 测试会话子命令的解析结果。
    ///
    /// - 手段：逐个解析 `add` / `list` / `show` / `update` / `remove`。
    /// - 判断：每个都得到预期的枚举值，且 `--workspace` 与位置参数互不干扰。
    #[test]
    fn parse_session_commands_() {
        let add = parse(argv_(&["session", "add", "--workspace", "w-1"])).expect("应当能解析");
        assert_eq!(
            add.command,
            Command::Session(SessionCommand::Add {
                workspace_id: "w-1".to_string(),
                title: None,
            })
        );

        let list = parse(argv_(&["session", "list", "--workspace", "w-1"])).expect("应当能解析");
        assert_eq!(
            list.command,
            Command::Session(SessionCommand::List {
                workspace_id: "w-1".to_string(),
            })
        );

        let show =
            parse(argv_(&["session", "show", "s-1", "--workspace", "w-1"])).expect("应当能解析");
        assert_eq!(
            show.command,
            Command::Session(SessionCommand::Show {
                workspace_id: "w-1".to_string(),
                session_id: "s-1".to_string(),
            })
        );

        let update = parse(argv_(&[
            "session",
            "update",
            "s-1",
            "--title",
            "改名",
            "--workspace",
            "w-1",
        ]))
        .expect("应当能解析");
        assert_eq!(
            update.command,
            Command::Session(SessionCommand::Update {
                workspace_id: "w-1".to_string(),
                session_id: "s-1".to_string(),
                title: "改名".to_string(),
            })
        );

        let remove =
            parse(argv_(&["session", "remove", "s-1", "--workspace", "w-1"])).expect("应当能解析");
        assert_eq!(
            remove.command,
            Command::Session(SessionCommand::Remove {
                workspace_id: "w-1".to_string(),
                session_id: "s-1".to_string(),
            })
        );
    }

    /// 测试缺参数、多参数、未知选项都会被明确拒绝。
    ///
    /// - 手段：分别构造七种坏输入。
    /// - 判断：每种都返回对应的 [`UsageError`] 变体——错误必须具体到"是哪个
    ///   选项/参数的问题"，而不是笼统的"解析失败"。
    #[test]
    fn parse_rejects_bad_input_() {
        let cases: Vec<(Vec<String>, UsageError)> = vec![
            (
                argv_(&["--nope"]),
                UsageError::UnknownOption("--nope".to_string()),
            ),
            (
                argv_(&["--runtime-dir"]),
                UsageError::MissingValue("--runtime-dir".to_string()),
            ),
            (
                argv_(&["workspace", "add", "--name", "笔记"]),
                UsageError::MissingOption("path"),
            ),
            (
                argv_(&["workspace", "show"]),
                UsageError::MissingOperand("<工作区标识>"),
            ),
            (
                argv_(&["workspace", "list", "多余"]),
                UsageError::UnexpectedArgument("多余".to_string()),
            ),
            (
                argv_(&["workspace", "update", "w-1"]),
                UsageError::Invalid("workspace update 至少要给出 --name 或 --path 之一"),
            ),
            (
                argv_(&["workspace", "frobnicate"]),
                UsageError::UnknownAction("workspace".to_string(), "frobnicate".to_string()),
            ),
            (
                argv_(&["notes", "list"]),
                UsageError::UnknownCommand("notes".to_string()),
            ),
            (
                argv_(&["session", "add", "--workspace"]),
                UsageError::MissingValue("--workspace".to_string()),
            ),
            (
                argv_(&["session"]),
                UsageError::MissingOperand("session <动作>"),
            ),
        ];

        for (argv, expected) in cases {
            let error = parse(argv.clone()).expect_err("应当被拒绝");
            assert_eq!(error, expected, "输入: {argv:?}");
        }
    }

    /// 测试 `--handshake-prompt` 的默认值与取值。
    ///
    /// - 手段：分别解析"不带该选项"、`stdio`、`none`，以及一个非法取值。
    /// - 判断：前两（三）者的 `handshake_prompt` 依次是 `None`（默认**不打扰**
    ///   stdout）、`Stdio`、`None`；非法取值返回 `InvalidOptionValue` 并带上原值。
    #[test]
    fn handshake_prompt_defaults_to_none_() {
        let default = parse(argv_(&["--runtime-dir", "/run/kb"])).expect("应当能解析");
        assert_eq!(default.handshake_prompt, HandshakePrompt::None);

        let stdio = parse(argv_(&["--handshake-prompt", "stdio"])).expect("应当能解析");
        assert_eq!(stdio.handshake_prompt, HandshakePrompt::Stdio);

        let none = parse(argv_(&["--handshake-prompt", "none"])).expect("应当能解析");
        assert_eq!(none.handshake_prompt, HandshakePrompt::None);

        let bad = parse(argv_(&["--handshake-prompt", "carrier-pigeon"])).expect_err("非法取值");
        assert_eq!(
            bad,
            UsageError::InvalidOptionValue {
                option: "handshake-prompt",
                value: "carrier-pigeon".to_string(),
            }
        );
    }
}
