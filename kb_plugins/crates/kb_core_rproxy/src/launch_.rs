//! 启动 `kb_core` 子进程，并读到它公布出来的 **IPC 端点文件名**。
//!
//! 这是整个网关里唯一的"系统层握手"动作：本模块负责"找得到"，
//! 协议层面的应用层握手（`Request::Hello`）在 `main.rs` 里、等远程客户端
//! 连上来之后才做。两个层面的分工见
//! `abs_kb_svc::v1::desktop::handshake_` 的模块文档。
//!
//! 通知格式是 `kb_core` 自己定的（`--handshake-prompt=stdio` 时往 stdout 打
//! 一行 JSON），**不属于 `abs_kb_svc` 的协议**——它是传输层约定，换实现可以改。

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use thiserror::Error;

/// 启动 `kb_core` 需要的东西。
#[derive(Debug, Clone)]
pub struct LaunchSpec {
    /// `kb_core` 可执行文件。
    pub kb_core: PathBuf,

    /// 运行时目录（IPC 端点名字文件放在这里）。
    pub runtime_dir: PathBuf,

    /// 知识库数据目录。
    pub storage_dir: PathBuf,
}

/// 已经启动起来、并且已经知道端点文件名的 `kb_core`。
#[derive(Debug)]
pub struct Launched {
    /// 子进程句柄；`Drop` 时会把它结束掉（见 [`Drop for Launched`]）。
    child_: Option<Child>,

    /// 子进程公布出来的端点名字文件。
    name_file_: PathBuf,
}

impl Launched {
    /// 端点名字文件的路径（`kb_core` 决定，名字里带日期与 UUID）。
    pub fn name_file(&self) -> &Path {
        &self.name_file_
    }

    /// 子进程的 pid（日志用）。
    pub fn pid(&self) -> Option<u32> {
        self.child_.as_ref().map(Child::id)
    }
}

/// `Drop` 时结束 `kb_core`：网关不在了，它没有理由继续活着。
///
/// 只是尽力而为（kill 失败不报错）；要优雅退出是后续的事。
impl Drop for Launched {
    fn drop(&mut self) {
        if let Some(child) = self.child_.as_mut() {
            log::info!("结束 kb_core 子进程 (pid={})", child.id());
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// 启动失败。
#[derive(Debug, Error)]
pub enum LaunchError {
    /// 找不到可执行文件。
    #[error("kb_core 可执行文件不可用: {}（用 --kb-core <路径> 指定）", .0.display())]
    KbCoreNotUsable(PathBuf),

    /// `spawn` 失败。
    #[error("启动 kb_core 失败: {0}")]
    Spawn(#[source] std::io::Error),

    /// 拿不到 stdout 管道（理论上不会发生，因为是我们自己 piped 的）。
    #[error("拿不到 kb_core 的 stdout 管道")]
    NoStdout,

    /// 读通知失败。
    #[error("读取 kb_core 的 stdio 通知失败: {0}")]
    ReadNotice(#[source] std::io::Error),

    /// 子进程还没来得及通知就退出了。
    #[error("kb_core 在给出 stdio 通知之前就结束了（检查它的 stderr 日志）")]
    ExitedEarly,

    /// 通知不是合法 JSON。
    #[error("kb_core 的 stdio 通知不是合法 JSON: {0}")]
    BadNotice(#[source] serde_json::Error),

    /// 通知里缺字段。
    #[error("kb_core 的 stdio 通知里没有 {0} 字段")]
    MissingField(&'static str),
}

/// 启动 `kb_core` 并把它的端点文件名读出来。
///
/// 这是**阻塞**调用（进程创建 + 读一行），应当从阻塞线程调用。
///
/// # Errors
///
/// 见 [`LaunchError`]。
pub fn launch(spec: &LaunchSpec) -> Result<Launched, LaunchError> {
    if !spec.kb_core.is_file() {
        return Err(LaunchError::KbCoreNotUsable(spec.kb_core.clone()));
    }

    let mut child = Command::new(&spec.kb_core)
        .arg("--handshake-prompt")
        .arg("stdio")
        .arg("--runtime-dir")
        .arg(&spec.runtime_dir)
        .arg("--storage-dir")
        .arg(&spec.storage_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(LaunchError::Spawn)?;

    let stdout = child.stdout.take().ok_or(LaunchError::NoStdout)?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let read = reader
        .read_line(&mut line)
        .map_err(LaunchError::ReadNotice)?;
    if read == 0 {
        return Err(LaunchError::ExitedEarly);
    }

    let notice: serde_json::Value =
        serde_json::from_str(line.trim()).map_err(LaunchError::BadNotice)?;
    let name_file = notice
        .get("ipc_name_file")
        .and_then(serde_json::Value::as_str)
        .ok_or(LaunchError::MissingField("ipc_name_file"))?;

    // 读完就把读端丢掉：kb_core 只往 stdout 写这一行，留着不读反而会在写满时
    // 把子进程堵住。
    drop(reader);

    log::info!(
        "kb_core 已启动 (pid={})，端点名字文件: {name_file}",
        child.id()
    );

    Ok(Launched {
        child_: Some(child),
        name_file_: PathBuf::from(name_file),
    })
}

/// 猜一个 `kb_core` 的位置：与本可执行文件同目录。
///
/// `cargo build` 会把 workspace 里所有 bin 放进同一个 `target/<profile>/`，
/// 因此开发时这条缺省值通常就是对的；生产部署可以用 `--kb-core` 覆盖。
pub fn default_kb_core_path() -> Option<PathBuf> {
    let current = std::env::current_exe().ok()?;
    let dir = current.parent()?;
    let candidate = dir.join("kb-core");
    candidate.is_file().then_some(candidate)
}
