//! 引导（rendezvous）：端点名字文件的原子发布与读取，以及客户端的重试连接。
//!
//! ipc-channel 的 `IpcOneShotServer` **只能接受一次连接**，因此多客户端要靠
//! 「每接受一个就重建并重发名字 + 客户端带重试」来维持。这一套的细节与实测
//! 数据见 `dev-notes/kb_svc_servo_ipc-20260917-1548.md` §1.1。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ipc_channel::ipc::IpcSender;

use super::connection_::Bootstrap;
use super::error_::ServoIpcError;

/// 端点名字文件的文件名。
pub const NAME_FILE: &str = "kb-core.ipc";

/// [`crate::Client::connect`] 缺省愿意等服务端端点多久。
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// 两次连接尝试之间的间隔。
const RETRY_GAP_: Duration = Duration::from_millis(20);

/// 端点名字文件的完整路径：`<runtime_dir>/kb-core.ipc`。
///
/// 这是**对外约定**：文件名与位置由服务端与客户端共同遵守，
/// 换实现（比如将来的 socket 版本）时各自定义自己的。
pub fn name_file_in(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(NAME_FILE)
}

/// 原子地公布端点名字。
///
/// 先写 `<名字>.ipc.tmp` 再 `rename`：读到的要么是旧名字、要么是新名字，
/// 不会是半个名字。与 `kb_core` 存储层用的是同一个手法。
pub(super) fn publish_name_(name_file: &Path, name: &str) -> Result<(), ServoIpcError> {
    let temp = name_file.with_extension("ipc.tmp");
    std::fs::write(&temp, name).map_err(|source| name_file_error_(name_file, source))?;
    std::fs::rename(&temp, name_file).map_err(|source| name_file_error_(name_file, source))
}

/// 撤销公布。
///
/// 文件本来就不存在算成功——撤销是幂等的，而且"名字文件不存在"正是
/// 客户端应当继续重试的状态。
pub(super) fn withdraw_name_(name_file: &Path) -> Result<(), ServoIpcError> {
    match std::fs::remove_file(name_file) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(name_file_error_(name_file, source)),
    }
}

/// 读取当前公布的名字。
///
/// 文件不存在、内容为空都返回 `None`——对调用方来说这两者是同一件事：
/// "现在还没有人在等连接"。
fn read_name_(name_file: &Path) -> Result<Option<String>, ServoIpcError> {
    match std::fs::read_to_string(name_file) {
        Ok(name) if !name.trim().is_empty() => Ok(Some(name.trim().to_string())),
        Ok(_) => Ok(None),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(name_file_error_(name_file, source)),
    }
}

/// 带重试地把引导发送端连到服务端。
///
/// **连接失败不是错误**：两次 `accept` 之间名字文件可能还是上一轮的（已经被
/// 消费掉）、也可能刚被撤下来，这些都只意味着"再等一会儿"。
/// 只有超过 `timeout` 还没连上才算失败。
pub(super) fn connect_with_retry_(
    name_file: &Path,
    timeout: Duration,
) -> Result<IpcSender<Bootstrap>, ServoIpcError> {
    let deadline = Instant::now() + timeout;

    loop {
        if let Some(name) = read_name_(name_file)? {
            match IpcSender::connect(name.clone()) {
                Ok(sender) => {
                    log::debug!("已连接到服务端端点 {name}");
                    return Ok(sender);
                }
                Err(error) => {
                    log::debug!(
                        "连接端点 {name} 失败（很可能是被别的客户端抢了），稍后重试: {error}"
                    );
                }
            }
        }

        if Instant::now() >= deadline {
            return Err(ServoIpcError::ConnectTimeout { timeout });
        }
        std::thread::sleep(RETRY_GAP_);
    }
}

/// 组装带路径的名字文件 I/O 错误。
fn name_file_error_(path: &Path, source: std::io::Error) -> ServoIpcError {
    ServoIpcError::NameFile {
        path: path.to_path_buf(),
        source,
    }
}
