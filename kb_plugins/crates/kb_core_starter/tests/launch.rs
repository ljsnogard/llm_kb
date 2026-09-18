//! `kb_core_starter` 的集成测试：用**假 kb_core**（一段 shell 脚本）覆盖
//! 启动/等待/取消的几条收场路径。
//!
//! 为什么不直接启动真的 `kb_core`：跨包拿不到别的包的 `CARGO_BIN_EXE_kb-core`
//! （那是定义了该 bin 的包的集成测试才有的环境变量），而且真进程会真的去建目录、
//! 开存储。这里关心的是**本 crate 自己的行为**：参数怎么传、通知怎么解析、
//! 取消之后子进程是不是真的没了——用一个行为可控的替身最合适。
//!
//! 整个文件只在 Unix 上编译：替身脚本用的是 `#!/bin/sh`。

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use abs_cancel::{CancelledToken, TrCancellationToken, TrMayCancel};
use futures_channel::oneshot;
use futures_lite::future::block_on;
use kb_core_starter::{LaunchError, LaunchSpec, kb_core_beside, start};

/// 串行化「写脚本 + 起进程」这一步的锁。
///
/// Linux 上 `fork` 出来的子进程会继承**同进程其它线程**打开的 fd：若 A 线程正在写
/// 某个脚本（写 fd 还没关），B 线程恰好 `fork`，B 的子进程就持有那个写 fd，
/// A 随后 exec 自己的脚本就会拿到 `ETXTBSY`（Text file busy）。
/// 这几个用例都拿"刚写出来的脚本"当假 kb_core，于是用一把锁把"写脚本 + 起进程"
/// 串起来，消掉这个**与生产代码无关**的内核竞态（实测：不加锁 40 次里会红 2 次）。
static SPAWN_LOCK_: Mutex<()> = Mutex::new(());

/// 取那把串行锁；中毒也照用——临界区里只有文件写入与 spawn，不会留下坏状态。
fn serialize_spawn_() -> MutexGuard<'static, ()> {
    SPAWN_LOCK_
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 在 `dir` 下造一个名为 `name` 的可执行 shell 脚本，正文是 `body`。
fn write_executable_(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("应当能写脚本");
    let mut permissions = std::fs::metadata(&path)
        .expect("应当能读元数据")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("应当能设执行位");
    path
}

/// 组装一份指向 `kb_core` 的 [`LaunchSpec`]，两个目录都落在临时目录里。
fn spec_with_(dir: &Path, kb_core: PathBuf) -> LaunchSpec {
    LaunchSpec {
        kb_core,
        runtime_dir: dir.join("run"),
        storage_dir: dir.join("data"),
    }
}

/// 判断某个 pid 是否还活着（`kill -0`）。
///
/// 子进程被 `kill` + `wait` 之后 pid 会被回收，因此"活着"为 `false` 正是
/// "没有留下孤儿"的证据。
fn process_exists_(pid: &str) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// 测试用的取消令牌：可以在任意时刻从任意线程触发一次。
///
/// `abs_cancel` 只提供"永不取消"与"一出生就已取消"两种令牌，而本 crate 要测的是
/// **等待中途被取消**，所以这里自带一个可触发的实现。
#[derive(Clone)]
struct TriggerToken_ {
    /// 是否已经触发过。
    cancelled_: Arc<AtomicBool>,

    /// 触发信号；`cancellation()` 被调用时把发送端存进来。
    sender_: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl TriggerToken_ {
    /// 造一个尚未触发的令牌。
    fn new_() -> Self {
        Self {
            cancelled_: Arc::new(AtomicBool::new(false)),
            sender_: Arc::new(Mutex::new(None)),
        }
    }

    /// 触发取消：置位并叫醒正在等它的 future。
    fn trigger_(&self) {
        self.cancelled_.store(true, Ordering::SeqCst);
        let sender = self.sender_.lock().expect("锁不应当中毒").take();
        if let Some(sender) = sender {
            let _ = sender.send(());
        }
    }
}

impl TrCancellationToken for TriggerToken_ {
    type Cancellation = oneshot::Receiver<()>;
    type ChildToken = TriggerToken_;

    fn is_cancelled(&self) -> bool {
        self.cancelled_.load(Ordering::SeqCst)
    }

    fn can_be_cancelled(&self) -> bool {
        true
    }

    fn child_token(&self) -> Self::ChildToken {
        self.clone()
    }

    fn cancellation(self) -> Self::Cancellation {
        let (sender, receiver) = oneshot::channel();
        if self.is_cancelled() {
            let _ = sender.send(());
            return receiver;
        }
        *self.sender_.lock().expect("锁不应当中毒") = Some(sender);
        receiver
    }
}

/// 测试正常路径：假 kb_core 打出一行合法通知，`start` 返回它公布的文件名与 pid。
///
/// - 手段：造一个脚本，先 `printf` 一行完整通知，再 `exec sleep 30` 保持存活；
///   用 `futures_lite::block_on` 驱动 `start(&spec)`（说明这个 future 不挑运行时）。
/// - 判断：返回 `Ok`；`name_file()` 等于脚本里写死的那个路径（说明参数确实传给了
///   子进程、通知也确实被解析）；`pid()` 是 `Some`（说明拿到的是一活的子进程）。
#[test]
fn start_reports_the_announced_name_file_() {
    let _serial = serialize_spawn_();
    let guard = tempfile::tempdir().expect("临时目录");
    let announced = guard.path().join("kb-20260918-abc.ipc");
    let body = format!(
        "printf '%s\\n' '{{\"event\":\"ipc_ready\",\"ipc_name_file\":\"{}\",\
         \"protocol_version\":1,\"pid\":1}}'\nexec sleep 30",
        announced.display()
    );
    let script = write_executable_(guard.path(), "kb-core-fake", &body);
    let spec = spec_with_(guard.path(), script);

    let launched = block_on(async { start(&spec).await }).expect("应当能启动");

    assert_eq!(launched.name_file(), announced.as_path());
    assert!(launched.pid().is_some());

    // 主动收工：`Launched` 的 `Drop` 负责结束子进程。
    drop(launched);
}

/// 测试子进程在给出通知之前就退出时，会被明确报成"提前结束"。
///
/// - 手段：脚本正文只写 `exit 0`，一行通知都不打。
/// - 判断：`start` 返回 [`LaunchError::ExitedEarly`]——而不是一直等到取消，
///   也不是拿一个空路径当成功。
#[test]
fn child_exiting_before_the_notice_is_reported_() {
    let _serial = serialize_spawn_();
    let guard = tempfile::tempdir().expect("临时目录");
    let script = write_executable_(guard.path(), "kb-core-fake", "exit 0");
    let spec = spec_with_(guard.path(), script);

    let outcome = block_on(async { start(&spec).await });

    assert!(
        matches!(outcome, Err(LaunchError::ExitedEarly)),
        "实际: {outcome:?}"
    );
}

/// 测试不是 JSON 的通知会被明确拒绝。
///
/// - 手段：脚本打一行"你好"（合法 UTF-8、但不是 JSON）。
/// - 判断：`start` 返回 [`LaunchError::BadNotice`]，且错误里带着 serde 的解析失败。
#[test]
fn non_json_notice_is_rejected_() {
    let _serial = serialize_spawn_();
    let guard = tempfile::tempdir().expect("临时目录");
    let script = write_executable_(guard.path(), "kb-core-fake", "printf '%s\\n' '你好'");
    let spec = spec_with_(guard.path(), script);

    let outcome = block_on(async { start(&spec).await });

    assert!(
        matches!(outcome, Err(LaunchError::BadNotice(_))),
        "实际: {outcome:?}"
    );
}

/// 测试可执行文件不可用时**不会**起进程。
///
/// - 手段：`kb_core` 指向一个不存在的路径。
/// - 判断：返回 [`LaunchError::KbCoreNotUsable`]；因为压根没 spawn，所以拿不到
///   任何 pid，也不会有子进程需要收尾。
#[test]
fn unusable_kb_core_is_rejected_before_spawn_() {
    let _serial = serialize_spawn_();
    let guard = tempfile::tempdir().expect("临时目录");
    let spec = spec_with_(guard.path(), guard.path().join("并不存在的 kb-core"));

    let outcome = block_on(async { start(&spec).await });

    assert!(
        matches!(outcome, Err(LaunchError::KbCoreNotUsable(_))),
        "实际: {outcome:?}"
    );
}

/// 测试已经取消的令牌下**不会**起进程。
///
/// - 手段：脚本一旦被执行就会写一个标记文件；用 `CancelledToken`（一出生就已取消）
///   走可取消路径。
/// - 判断：返回 [`LaunchError::Cancelled`]，且标记文件**不存在**——说明取消发生在
///   spawn 之前，"取消不留下副作用"这条成立。
#[test]
fn already_cancelled_token_never_spawns_() {
    let _serial = serialize_spawn_();
    let guard = tempfile::tempdir().expect("临时目录");
    let marker = guard.path().join("spawned.marker");
    let body = format!("echo spawned > '{}'\nexec sleep 30", marker.display());
    let script = write_executable_(guard.path(), "kb-core-fake", &body);
    let spec = spec_with_(guard.path(), script);

    let outcome = block_on(async { start(&spec).may_cancel_with(CancelledToken::new()).await });

    assert!(outcome.is_err(), "实际: {outcome:?}");
    assert!(outcome.unwrap_err().is_cancelled());
    assert!(!marker.exists(), "已经取消时不应起进程");
}

/// 测试**等待中途**被取消：立即收手，并且把已经起来的子进程结束掉。
///
/// - 手段：脚本先把自己的 pid 写进文件，再 `exec sleep 30`（始终不打通知）。
///   另一个线程轮询到 pid 文件之后触发取消令牌；主线程用 `block_on` 等
///   `start(&spec).may_cancel_with(token)`。
/// - 判断：三件事同时成立——
///   1. 结果是 [`LaunchError::Cancelled`]（不是"等满 30 秒"）；
///   2. 从触发到返回的耗时远小于脚本的存活时间（取消真的生效）；
///   3. `kill -0` 查不到那个 pid（取消把子进程收掉了，没有留下孤儿）。
#[test]
fn cancelling_the_wait_kills_the_child_() {
    let _serial = serialize_spawn_();
    let guard = tempfile::tempdir().expect("临时目录");
    let pid_file = guard.path().join("child.pid");
    let body = format!("echo $$ > '{}'\nexec sleep 30", pid_file.display());
    let script = write_executable_(guard.path(), "kb-core-fake", &body);
    let spec = spec_with_(guard.path(), script);

    let token = TriggerToken_::new_();
    let trigger = token.clone();
    let watcher_pid_file = pid_file.clone();
    let watcher = std::thread::spawn(move || {
        // 等脚本把自己的 pid 写出来，再触发取消——这样取消一定发生在"等待通知"期间。
        let deadline = Instant::now() + Duration::from_secs(10);
        while !watcher_pid_file.exists() {
            assert!(Instant::now() < deadline, "假 kb_core 没有写出 pid");
            std::thread::sleep(Duration::from_millis(10));
        }
        trigger.trigger_();
    });

    let started_at = Instant::now();
    let outcome = block_on(async { start(&spec).may_cancel_with(token).await });
    let elapsed = started_at.elapsed();
    watcher.join().expect("触发线程应当正常结束");

    assert!(
        matches!(outcome, Err(LaunchError::Cancelled)),
        "实际: {outcome:?}"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "取消应当立刻收手，实际等了 {elapsed:?}"
    );

    let pid = std::fs::read_to_string(&pid_file)
        .expect("应当有 pid 文件")
        .trim()
        .to_string();
    assert!(!process_exists_(&pid), "取消之后子进程 {pid} 应当已被结束");
}

/// 测试 `kb_core_beside` 能按目录找到 `kb-core`，找不到时返回 `None`。
///
/// - 手段：在一个临时目录里放一个名为 `kb-core` 的可执行文件，另取一个空目录。
/// - 判断：前者返回那个路径；后者返回 `None`。这条正是"打包出来的 App 必须靠配置
///   指定路径"的兜底能力：调用方自己决定拿哪个目录当基准。
#[test]
fn kb_core_beside_finds_the_executable_() {
    let guard = tempfile::tempdir().expect("临时目录");
    let expected = write_executable_(guard.path(), "kb-core", "exit 0");
    let empty = tempfile::tempdir().expect("临时目录");

    assert_eq!(kb_core_beside(guard.path()), Some(expected));
    assert_eq!(kb_core_beside(empty.path()), None);
}
