//! 启动 `kb_core` 子进程，并**异步地**等它公布 IPC 端点文件名。
//!
//! # 形状
//!
//! ```text
//! start(&spec).await                     ← future：不阻塞执行器线程
//!   │
//!   ├─ 第 1 次 poll：spawn 子进程 + 起一条专职线程读 stdout 那一行
//!   │                （`Command::spawn` 是短系统调用；真正的"等"不在这里）
//!   └─ 之后：只轮询「通知完成量」与「取消令牌」
//!                │
//!                └─ 通知到了 → Launched（子进程所有权交出去）
//!                   取消/出错/future 被丢弃 → ChildGuard_ 结束子进程
//! ```
//!
//! # 为什么"等"要收敛到别处
//!
//! `read_line` 会一直阻塞到子进程写出那一行。把它留在 future 的 `poll` 里，
//! `current_thread` 一类的执行器就整个卡住了（`abs_kb_svc` README §5 第 1、2 条：
//! future 里禁止阻塞）。所以这里用一条专职线程 + `futures_channel::oneshot`：
//! 与本仓库 `kb_svc_servo_ipc::Client` 的"路由线程 + 完成量"是同一种结构。
//!
//! # 取消与子进程归属
//!
//! 只要 `kb_core` 是本模块起来的，它就不会在"等待失败"之后变成孤儿：
//! 子进程从一出生就被 [`ChildGuard_`] 持有，而守卫在 `Drop` 里 `kill` + `wait`。
//! 于是三条收场路径共用同一段收尾逻辑：
//!
//! | 收场 | 子进程 |
//! | :--- | :--- |
//! | 成功 | 所有权转给 [`Launched`]，它在 `Drop` 时结束进程 |
//! | 取消 / 读通知失败 | 守卫被丢弃 → 结束进程 |
//! | future 被直接丢弃 | 同上（状态机里的守卫随 future 一起 drop） |

use std::future::Future;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};

use abs_cancel::TrCancellationToken;
use futures_channel::oneshot;
use gen_mcf2::gen_may_cancel_future;

use crate::error_::LaunchError;
use crate::notice_::parse_name_file_;

/// 启动 `kb_core` 需要的东西。
///
/// 三个字段都**必须由调用方给出**：`kb_core` 的位置没有可靠的缺省值
/// （见 [`default_kb_core_path`] 的说明），两个目录则决定了它挂哪个运行时目录、
/// 读哪份知识库。
#[derive(Debug, Clone)]
pub struct LaunchSpec {
    /// `kb_core` 可执行文件。
    pub kb_core: PathBuf,

    /// 运行时目录（IPC 端点名字文件放在这里）。
    pub runtime_dir: PathBuf,

    /// 知识库数据目录。
    pub storage_dir: PathBuf,
}

/// 已经启动起来、并且已经知道 IPC 端点文件名的 `kb_core`。
///
/// 它**拥有**那个子进程：`Drop` 时会把它结束掉。想让它活得更久，就把这个值
/// 活得比调用栈长（例如放进客户端自己的连接管理器）。
#[derive(Debug)]
pub struct Launched {
    /// 子进程守卫；`Drop` 时结束子进程。
    child_: ChildGuard_,

    /// 子进程公布出来的端点名字文件。
    name_file_: PathBuf,
}

impl Launched {
    /// 端点名字文件的路径（由 `kb_core` 决定，名字里带日期与 UUID）。
    ///
    /// 它是**文件名**不是"现在就能连上"：文件内容（当前可连的端点名）要等
    /// `kb_core` 真正开始 accept 才会写进去，连接方本来就有重试。
    pub fn name_file(&self) -> &Path {
        &self.name_file_
    }

    /// 子进程的 pid（日志用）。
    pub fn pid(&self) -> Option<u32> {
        self.child_.pid_()
    }
}

/// 启动 `kb_core` 并等它公布 IPC 端点文件名。
///
/// 返回的 future **与异步运行时无关**，并且可以取消：
///
/// ```text
/// start(&spec).await                          // 不可取消：一直等
/// start(&spec).may_cancel_with(token).await   // 可取消：令牌触发即收手
/// ```
///
/// 取消（或出错、或直接丢弃这个 future）都会**结束已经起来的子进程**，
/// 调用方不需要再收拾它。
///
/// # Errors
///
/// - [`LaunchError::Cancelled`]：等待被取消（子进程已被结束）；
/// - [`LaunchError::KbCoreNotUsable`]：可执行文件不可用（**不会**起进程）；
/// - [`LaunchError::Spawn`] / [`LaunchError::NoStdout`] / [`LaunchError::NoticeThread`]：
///   起进程、拿管道、起读线程失败；
/// - [`LaunchError::ExitedEarly`]：子进程没打通知就退了；
/// - [`LaunchError::ReadNotice`]：读 stdout 失败；
/// - [`LaunchError::BadNotice`] / [`LaunchError::MissingField`]：通知不可用。
pub fn start<'s>(spec: &'s LaunchSpec) -> StartAsync<'s, 's> {
    StartAsync::new(spec)
}

/// [`start`] 的实现体；宏负责生成 [`StartAsync`] / `StartFuture` 与两条路径。
///
/// 保持私有：对外只有 [`start`] 一个入口，调用方用 `.may_cancel_with(…)`
/// 表达取消，不必自己构造这个函数要的令牌参数。
#[gen_may_cancel_future(Start, pub)]
async fn start_async<'s, C>(spec: &'s LaunchSpec, cancel: C) -> Result<Launched, LaunchError>
where
    C: TrCancellationToken,
{
    // 已经取消就没必要起进程——这条也保证了"取消时不留下任何副作用"。
    if cancel.is_cancelled() {
        return Err(LaunchError::Cancelled);
    }

    let mut child = spawn_child_(spec)?;
    let stdout = child.stdout.take().ok_or(LaunchError::NoStdout)?;

    // 进程一旦起来就交给守卫：取消、出错、future 被丢弃这三种收场都会把它结束掉。
    let guard = ChildGuard_::new_(child);

    let (sender, receiver) = oneshot::channel::<Result<PathBuf, LaunchError>>();
    std::thread::Builder::new()
        .name("kb-core-starter-notice".to_string())
        .spawn(move || {
            let outcome = read_notice_(stdout);
            // 接收端可能已经走开（被取消 / future 被丢弃）。送不到就把结果丢掉：
            // 若那是个 `Launched`，它的 `Drop` 会顺手结束子进程。
            let _ = sender.send(outcome);
        })
        .map_err(LaunchError::NoticeThread)?;

    // 取消时这里返回 `Err`，`guard` 随后随本次调用一起被丢弃 → 结束子进程。
    let name_file = wait_notice_(receiver, cancel).await?;

    let launched = Launched {
        child_: guard,
        name_file_: name_file,
    };
    log::info!(
        "kb_core 已启动 (pid={:?})，端点名字文件: {}",
        launched.pid(),
        launched.name_file().display()
    );

    Ok(launched)
}

/// 猜一个 `kb_core` 的位置：与本可执行文件同目录。
///
/// `cargo build` 会把 workspace 里所有 bin 放进同一个 `target/<profile>/`，
/// 因此开发时这条缺省值通常是对的。
///
/// ⚠️ **打包出来的 GUI 程序不能依赖它**：Flutter 应用里 `current_exe()` 是
/// Flutter runner，`kb-core` 既不在它旁边、也不会自动被带进 bundle。那种场景
/// 必须把路径写进配置（见 [`LaunchSpec::kb_core`]）。
pub fn default_kb_core_path() -> Option<PathBuf> {
    let current = std::env::current_exe().ok()?;
    kb_core_beside(current.parent()?)
}

/// 在指定目录里找 `kb_core` 可执行文件（`kb-core`，Windows 上是 `kb-core.exe`）。
///
/// 把"基准目录"作为参数而不是在函数里读 `current_exe()`，是为了让调用方
/// （打包目录、安装目录、测试夹具）能复用它，也便于确定性地测试。
pub fn kb_core_beside(dir: &Path) -> Option<PathBuf> {
    let candidate = dir.join(kb_core_file_name_());
    candidate.is_file().then_some(candidate)
}

/// `kb_core` 可执行文件的名字（Windows 带 `.exe`）。
fn kb_core_file_name_() -> &'static str {
    if cfg!(windows) {
        "kb-core.exe"
    } else {
        "kb-core"
    }
}

/// 起进程。可执行文件不可用时**在起进程之前**就报错。
fn spawn_child_(spec: &LaunchSpec) -> Result<Child, LaunchError> {
    if !spec.kb_core.is_file() {
        return Err(LaunchError::KbCoreNotUsable(spec.kb_core.clone()));
    }

    Command::new(&spec.kb_core)
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
        .map_err(LaunchError::Spawn)
}

/// 读一行通知并解析出端点文件名；**阻塞**，只应当在专职线程上调用。
fn read_notice_(stdout: ChildStdout) -> Result<PathBuf, LaunchError> {
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let read = reader
        .read_line(&mut line)
        .map_err(LaunchError::ReadNotice)?;
    if read == 0 {
        return Err(LaunchError::ExitedEarly);
    }

    let name_file = parse_name_file_(&line)?;

    // 读完就把读端丢掉（`reader` 在这里离开作用域）：`kb_core` 只往 stdout 写这一行，
    // 留着不读反而会在管道写满时把子进程堵住。
    Ok(name_file)
}

/// 等通知，或在取消令牌触发时收手。
///
/// 与本仓库 `kb_svc_servo_ipc::Client::await_reply_` 是同一套写法：只轮询完成量
/// 与取消 future，不在这里做任何阻塞调用。
async fn wait_notice_<C>(
    receiver: oneshot::Receiver<Result<PathBuf, LaunchError>>,
    cancel: C,
) -> Result<PathBuf, LaunchError>
where
    C: TrCancellationToken,
{
    let cancellation = cancel.cancellation();
    let mut receiver = receiver;
    let mut cancellation = std::pin::pin!(cancellation);

    std::future::poll_fn(move |context| {
        if let std::task::Poll::Ready(outcome) = std::pin::Pin::new(&mut receiver).poll(context) {
            return std::task::Poll::Ready(match outcome {
                Ok(result) => result,
                // 完成量被丢弃 = 读线程没送结果就结束了（通常意味着它 panic 了）。
                Err(_dropped) => Err(LaunchError::NoticeThreadEnded),
            });
        }
        if std::pin::Pin::new(&mut cancellation)
            .poll(context)
            .is_ready()
        {
            return std::task::Poll::Ready(Err(LaunchError::Cancelled));
        }
        std::task::Poll::Pending
    })
    .await
}

/// 子进程守卫：无论因为哪条路径被丢弃，都结束它。
///
/// 只是尽力而为（kill 失败不报错）；要优雅退出是后续的事——`kb_core` 目前也
/// 没有安装信号处理器。
#[derive(Debug)]
struct ChildGuard_ {
    /// 子进程句柄。
    child_: Option<Child>,
}

impl ChildGuard_ {
    /// 接管一个刚起来的子进程。
    fn new_(child: Child) -> Self {
        Self {
            child_: Some(child),
        }
    }

    /// 子进程 pid。
    fn pid_(&self) -> Option<u32> {
        self.child_.as_ref().map(Child::id)
    }
}

impl Drop for ChildGuard_ {
    fn drop(&mut self) {
        if let Some(child) = self.child_.as_mut() {
            log::debug!("结束 kb_core 子进程 (pid={})", child.id());
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
