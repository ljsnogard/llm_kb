//! 引导（rendezvous）：端点名字文件的**命名**、发布与查找，以及客户端的重试连接。
//!
//! # 文件名由 `kb_core` 决定，遵循「日期 + UUID」
//!
//! 端点名字文件叫 `<runtime_dir>/kb-<YYYYMMDD>-<uuid-v4>.ipc`：
//!
//! ```text
//! /run/user/1000/llm_kb/kb-20260917-f9c628f955594c1fa73965bcac42f091.ipc
//!                       └─ 日期 ─┘└──────── UUID v4 ────────┘
//! ```
//!
//! 这是沿用 `kb_svc_salvo` 时期 `plugin_socket` 的约定（那里的后缀是 `.sock`）。
//! 保留它的理由是**与传输实现无关**：`kb_core` 必然长期需要"本机 IPC"这件事，
//! 而本机 IPC 无非是 Unix domain socket 或同等机制（Windows 上也有命名管道）。
//! 所以"每次启动用日期 + UUID 生成一个独一无二的名字"这条约定应当属于
//! `kb_core` 这一层，而不是由某个传输库替我们决定——换掉 `ipc-channel`
//! 换不掉这条约定。
//!
//! 文件名的语义是**服务端实例标识**：
//!
//! - 一个 `kb_core` 进程对应一个名字文件，进程活着它就在；
//! - 文件**内容**是当前可连接的传输端点名；没有在等连接时写空串。
//!
//! > ⚠️ 与 `ipc-channel` 的边界：`IpcOneShotServer` 内部把自己绑在
//! > `<系统临时目录>/…/socket` 上，且**没有**提供指定路径的接口
//! > （`OsIpcOneShotServer::new()` 固定用 `tempdir()/socket`，
//! > `OsIpcReceiver::from_fd` 是私有的）。因此"操作系统层面的 socket 路径"
//! > 目前仍由 `ipc-channel` 决定；**由 `kb_core` 决定并对外公布的是这个名字文件**。
//! > 将来若要让 socket 本身也落在我们指定的路径上，就得自己建监听 socket
//! > （`libc` + `SOCK_SEQPACKET`）而不再使用 `IpcOneShotServer`——
//! > 那是一次独立决策，见 `dev-notes/kb_svc_servo_ipc-20260917-1548.md`。
//!
//! # 多客户端
//!
//! `IpcOneShotServer` **只能接受一次连接**，所以多客户端靠
//! 「每接受一个就重建并重发端点 + 客户端带重试」维持。细节与实测数据见
//! `dev-notes/kb_svc_servo_ipc-20260917-1548.md` §1.1。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use ipc_channel::ipc::IpcSender;
use uuid::Uuid;

use super::connection_::Bootstrap;
use super::error_::ServoIpcError;

/// 端点名字文件的文件名前缀。
pub const NAME_FILE_PREFIX: &str = "kb-";

/// 端点名字文件的扩展名（不含点）。
pub const NAME_FILE_EXTENSION: &str = "ipc";

/// [`crate::Client::connect`] 缺省愿意等服务端端点多久。
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// 两次连接尝试之间的间隔。
const RETRY_GAP_: Duration = Duration::from_millis(20);

/// 生成一个「本次启动专属」的端点名字文件路径：
/// `<runtime_dir>/kb-<YYYYMMDD>-<uuid-v4>.ipc`。
///
/// 只生成路径，不碰磁盘——`kb_core` 会先清掉上一次运行留下的名字文件，
/// 再在真正等连接时把端点名写进去。
pub fn new_name_file_in(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(format!(
        "{NAME_FILE_PREFIX}{}-{}.{NAME_FILE_EXTENSION}",
        today_utc_compact(),
        Uuid::new_v4().simple()
    ))
}

/// 在运行时目录里找出**当前可以连的**那个名字文件。
///
/// 判据是"内容非空"（空串表示这个服务端此刻没有在等连接）。理论上一个运行时
/// 目录只挂一个 `kb_core`，因此通常至多命中一个；万一有残留，取修改时间最新的。
///
/// # Errors
///
/// 目录读不了时返回 [`ServoIpcError::RuntimeDir`]。
pub fn find_name_file_in(runtime_dir: &Path) -> Result<Option<PathBuf>, ServoIpcError> {
    let mut newest: Option<(SystemTime, PathBuf)> = None;

    for path in list_name_files_(runtime_dir)? {
        if read_name_(&path)?.is_none() {
            continue;
        }
        let modified = std::fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if newest
            .as_ref()
            .is_none_or(|(current, _)| modified > *current)
        {
            newest = Some((modified, path));
        }
    }

    Ok(newest.map(|(_, path)| path))
}

/// 清掉运行时目录里**全部**名字文件。
///
/// `kb_core` 启动时调用：上一次进程无论是正常退出还是被强杀，留下来的名字文件
/// 都只可能指向一个已经消失的端点，留着只会让客户端对着死名字重试到超时。
/// 既然本进程就是服务端，启动时那里面的名字一定无效。
///
/// 一个运行时目录只应当挂**一个** `kb_core`；两个实例会互相清名字。
///
/// 单个文件删不掉只记警告，不让启动失败——它最多让客户端多试一次。
pub(super) fn clear_stale_name_files_(runtime_dir: &Path) -> Result<(), ServoIpcError> {
    for path in list_name_files_(runtime_dir)? {
        if let Err(source) = std::fs::remove_file(&path) {
            log::warn!("清理残留端点名字文件失败 {}: {source}", path.display());
        }
    }
    Ok(())
}

/// 把端点名写进名字文件（原子：先写 `.tmp` 再 `rename`）。
pub(super) fn publish_name_(name_file: &Path, name: &str) -> Result<(), ServoIpcError> {
    write_name_(name_file, name)
}

/// 把名字文件的内容清空：表示"当前没有在等连接的端点"。
///
/// 刻意**保留文件本身**——它是这个 `kb_core` 实例的标识，进程活着就应当在；
/// 客户端只看内容，不看文件在不在。
pub(super) fn clear_name_(name_file: &Path) -> Result<(), ServoIpcError> {
    write_name_(name_file, "")
}

/// 读取名字文件里的端点名；空内容或文件不存在都返回 `None`。
pub(super) fn read_name_(name_file: &Path) -> Result<Option<String>, ServoIpcError> {
    match std::fs::read_to_string(name_file) {
        Ok(name) if !name.trim().is_empty() => Ok(Some(name.trim().to_string())),
        Ok(_) => Ok(None),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(name_file_error_(name_file, source)),
    }
}

/// 带重试地把引导发送端连到服务端。
///
/// **连接失败不是错误**：端点可能刚被别的客户端抢走、也可能服务端正在换下一个
/// 端点，这些都只意味着"再等一会儿"。只有超过 `timeout` 还没连上才算失败。
pub(super) fn connect_with_retry_(
    runtime_dir: &Path,
    timeout: Duration,
) -> Result<IpcSender<Bootstrap>, ServoIpcError> {
    let deadline = Instant::now() + timeout;

    loop {
        if let Some(name_file) = find_name_file_in(runtime_dir)?
            && let Some(name) = read_name_(&name_file)?
        {
            match IpcSender::connect(name.clone()) {
                Ok(sender) => {
                    log::debug!("已连接到服务端端点 {name}（{}）", name_file.display());
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

/// 列出运行时目录里全部形如 `kb-*.ipc` 的文件。
fn list_name_files_(runtime_dir: &Path) -> Result<Vec<PathBuf>, ServoIpcError> {
    let entries = match std::fs::read_dir(runtime_dir) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(ServoIpcError::RuntimeDir {
                path: runtime_dir.to_path_buf(),
                source,
            });
        }
    };

    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| ServoIpcError::RuntimeDir {
            path: runtime_dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(NAME_FILE_PREFIX)
            && path.extension().and_then(|extension| extension.to_str())
                == Some(NAME_FILE_EXTENSION)
        {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// 原子地写名字文件。
fn write_name_(name_file: &Path, name: &str) -> Result<(), ServoIpcError> {
    let temp = name_file.with_extension("ipc.tmp");
    std::fs::write(&temp, name).map_err(|source| name_file_error_(name_file, source))?;
    std::fs::rename(&temp, name_file).map_err(|source| name_file_error_(name_file, source))
}

/// 以 `YYYYMMDD` 形式返回当前 UTC 日期。
///
/// 沿用 `kb_svc_salvo::plugin_socket` 的实现（那里用来拼 socket 文件名）：
/// 纯整数运算，不引入 `chrono` / `time`，也不受本地时区与闰秒影响。
fn today_utc_compact() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);

    let (year, month, day) = civil_from_days_(secs.div_euclid(86_400));

    format!("{year:04}{month:02}{day:02}")
}

/// 把「自 1970-01-01 起的天数」转换为公历年月日。
///
/// 采用 Howard Hinnant 的 `civil_from_days` 算法。
fn civil_from_days_(days: i64) -> (i64, u32, u32) {
    // 把纪元原点从 1970-01-01 挪到 0000-03-01，让闰年周期落在 400 年的整数倍上。
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };

    (year, month, day)
}

/// 组装带路径的名字文件 I/O 错误。
fn name_file_error_(path: &Path, source: std::io::Error) -> ServoIpcError {
    ServoIpcError::NameFile {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests_ {
    use super::*;

    /// 测试生成的名字文件形如 `kb-<YYYYMMDD>-<32位小写十六进制>.ipc`。
    ///
    /// - 手段：在同一个运行时目录上连生成两个名字。
    /// - 判断：都匹配「前缀 + 8 位数字 + `-` + 32 位十六进制 + `.ipc`」，
    ///   且两者不相同——日期段固定、UUID 段每次不同，正是"每次启动独一无二"
    ///   这条约定的写法。
    #[test]
    fn generated_name_file_follows_date_and_uuid_convention_() {
        let root = Path::new("/run/kb");
        let first = new_name_file_in(root);
        let second = new_name_file_in(root);

        for path in [&first, &second] {
            assert_eq!(path.parent(), Some(root));
            let name = path.file_name().expect("应当有文件名").to_string_lossy();
            let body = name
                .strip_prefix("kb-")
                .and_then(|rest| rest.strip_suffix(".ipc"))
                .unwrap_or_else(|| panic!("实际文件名: {name}"));
            let mut parts = body.split('-');
            let date = parts.next().expect("应当有日期段");
            let uuid = parts.next().expect("应当有 UUID 段");
            assert_eq!(parts.next(), None, "只应当有两段: {body}");

            assert_eq!(date.len(), 8, "日期段应当是 YYYYMMDD: {date}");
            assert!(date.chars().all(|ch| ch.is_ascii_digit()), "日期段: {date}");
            assert_eq!(uuid.len(), 32, "UUID 段应当是 32 位: {uuid}");
            assert!(
                uuid.chars()
                    .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()),
                "UUID 段: {uuid}"
            );
        }

        assert_ne!(first, second, "每次生成的名字应当不同");
    }

    /// 测试日期算法与已知日期一致。
    ///
    /// - 手段：对 `civil_from_days_` 喂入几个已知的"自纪元起的天数"。
    /// - 判断：结果与公历日期一致（含闰年 2024-02-29）。
    #[test]
    fn civil_from_days_matches_known_dates_() {
        assert_eq!(civil_from_days_(0), (1970, 1, 1));
        assert_eq!(civil_from_days_(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days_(19_782), (2024, 2, 29));
    }

    /// 测试名字文件的发布、读取与清空。
    ///
    /// - 手段：在临时目录里造一个名字文件，依次 publish → read → clear → read。
    /// - 判断：publish 之后能读回端点名、`find_name_file_in` 能找到它；
    ///   clear 之后读出 `None` 且找不到，但**文件仍在**（它是服务端实例的标识）。
    #[test]
    fn publish_read_and_clear_round_trip_() {
        let guard = tempfile::tempdir().expect("临时目录");
        let name_file = new_name_file_in(guard.path());

        publish_name_(&name_file, "端点甲").expect("应当能发布");
        assert_eq!(
            read_name_(&name_file).expect("应当能读"),
            Some("端点甲".to_string())
        );
        assert_eq!(
            find_name_file_in(guard.path()).expect("应当能找"),
            Some(name_file.clone())
        );

        clear_name_(&name_file).expect("应当能清空");
        assert_eq!(read_name_(&name_file).expect("应当能读"), None);
        assert!(name_file.is_file(), "清空只写空内容，文件本身应当还在");
        assert_eq!(find_name_file_in(guard.path()).expect("应当能找"), None);
    }

    /// 测试启动时清理残留名字文件，但不碰无关文件。
    ///
    /// - 手段：先放两个 `kb-*.ipc` 与一个无关文件，再调用清理。
    /// - 判断：两个 `kb-*.ipc` 都被删掉，无关文件保留。
    #[test]
    fn stale_name_files_are_cleared_but_other_files_survive_() {
        let guard = tempfile::tempdir().expect("临时目录");
        let first = new_name_file_in(guard.path());
        let second = new_name_file_in(guard.path());
        let unrelated = guard.path().join("notes.txt");
        std::fs::write(&first, "甲").expect("写");
        std::fs::write(&second, "乙").expect("写");
        std::fs::write(&unrelated, "别删我").expect("写");

        clear_stale_name_files_(guard.path()).expect("清理应当成功");

        assert!(!first.exists());
        assert!(!second.exists());
        assert!(unrelated.is_file(), "无关文件不应当被清理");
    }
}
