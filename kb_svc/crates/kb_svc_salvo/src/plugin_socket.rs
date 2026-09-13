//! 插件通道 Unix domain socket 的路径生成与生命周期管理。
//!
//! # 约定
//!
//! socket 文件名**不由命令行参数或环境变量指定**，而是每次启动时由
//! 「日期 + UUID v4」拼接生成，形如：
//!
//! ```text
//! <runtime_dir>/kb-20260913-3f2b9c1d4e5a4b7c8d9e0f1a2b3c4d5e.sock
//! ```
//!
//! 这样做的目的：
//!
//! 1. 每次启动得到互不冲突的名字，历史遗留的 socket 文件不会影响本次启动；
//! 2. 文件名自带日期，便于事后排查「这个 socket 是哪天哪个进程留下的」；
//! 3. 文件名不作为对外契约，调用方（尤其是插件子进程）应当从父进程处**被动获得**
//!    该路径，而不是自己拼装。
//!
//! 插件子进程通过 `--socket <path>` 命令行参数与 `LLM_KB_PLUGIN_SOCKET`
//! 环境变量两种方式拿到同一个路径。

use std::{
    fs::DirBuilder,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};

use uuid::Uuid;

use crate::error::{KbSvcError, KbSvcResult};

/// 运行时目录在 `$XDG_RUNTIME_DIR` 缺失时的回退目录名。
const FALLBACK_DIR_NAME: &str = "llm_kb";

/// socket 文件名前缀。
const SOCKET_FILE_PREFIX: &str = "kb";

/// socket 文件后缀。
const SOCKET_FILE_SUFFIX: &str = "sock";

/// socket 文件的权限：仅所有者可读写。
const SOCKET_FILE_MODE: u32 = 0o600;

/// 运行时目录的权限：仅所有者可进入。
const RUNTIME_DIR_MODE: u32 = 0o700;

/// 运行时目录的默认位置。
///
/// 优先使用 `$XDG_RUNTIME_DIR/llm_kb`；该变量缺失（例如容器、root 直接登录）时
/// 回退到系统临时目录下的 `llm_kb`。
pub fn default_runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(FALLBACK_DIR_NAME)
}

/// 生成一个新的 socket 文件路径（**不创建文件**）。
///
/// - `date`：形如 `20260913` 的日期字符串，由调用方提供，便于测试与替换实现。
///
/// 返回路径的父目录为 `runtime_dir`，文件名格式为
/// `kb-<date>-<uuid-v4-simple>.sock`。
pub fn generate_path(runtime_dir: &Path, date: &str) -> PathBuf {
    let uuid = Uuid::new_v4().simple();

    runtime_dir.join(format!(
        "{SOCKET_FILE_PREFIX}-{date}-{uuid}.{SOCKET_FILE_SUFFIX}"
    ))
}

/// 确保运行时目录存在，并把权限收紧到 `0700`。
///
/// 目录已存在时不会修改其权限，避免覆盖部署方的显式配置。
pub fn ensure_runtime_dir(runtime_dir: &Path) -> KbSvcResult<()> {
    if runtime_dir.is_dir() {
        return Ok(());
    }

    let mut builder = DirBuilder::new();
    builder.recursive(true).mode(RUNTIME_DIR_MODE);

    builder.create(runtime_dir).map_err(KbSvcError::Io)
}

/// 把 socket 文件的权限收紧到 `0600`。
///
/// Salvo 的 `UnixListener` 也支持在 bind 时设置权限；这里额外做一次是为了让
/// 「路径生成」这一层自带安全默认值，即使调用方忘了设置也不会把 socket 暴露给
/// 同机的其他用户。
pub fn restrict_socket_permissions(path: &Path) -> KbSvcResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = std::fs::Permissions::from_mode(SOCKET_FILE_MODE);

    std::fs::set_permissions(path, permissions).map_err(KbSvcError::Io)
}

/// 把「日期 + UUID」拼成的语义收敛到一处：给定运行时目录，生成一个可用的 socket 路径。
///
/// 与 [`generate_path`] 的区别是这里内部使用当前 UTC 日期 (`YYYYMMDD`)，
/// 因此本函数是生产路径；[`generate_path`] 保留给需要固定日期的测试。
pub fn generate_for_today(runtime_dir: &Path) -> PathBuf {
    generate_path(runtime_dir, &today_utc_compact())
}

/// 以 `YYYYMMDD` 形式返回当前 UTC 日期。
fn today_utc_compact() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);

    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));

    format!("{year:04}{month:02}{day:02}")
}

/// 把「自 1970-01-01 起的天数」转换为公历年月日。
///
/// 采用 Howard Hinnant 的 `civil_from_days` 算法；它是纯整数运算，
/// 不引入 `chrono` / `time` 依赖，也不会因为本地时区或闰秒产生歧义。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
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

/// 删除一个 socket 文件；文件不存在时视为成功。
pub fn remove_socket_file(path: &Path) -> KbSvcResult<()> {
    match std::fs::remove_file(path) {
        Ok(()) => {
            log::debug!("removed socket file: {}", path.display());
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(KbSvcError::Io(err)),
    }
}

/// 在 `Drop` 时删除 socket 文件的守卫。
///
/// 正式实现应当在服务端优雅退出时显式调用 [`SocketFileGuard::remove`]；
/// `Drop` 只是进程异常退出路径上的兜底。
#[derive(Debug)]
pub struct SocketFileGuard {
    path: PathBuf,
}

impl SocketFileGuard {
    /// 为一个已经 bind 成功的 socket 路径创建守卫。
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 返回被守卫的路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 显式删除 socket 文件（幂等）。
    pub fn remove(&self) -> KbSvcResult<()> {
        remove_socket_file(&self.path)
    }
}

impl Drop for SocketFileGuard {
    fn drop(&mut self) {
        if let Err(err) = remove_socket_file(&self.path) {
            log::warn!(
                "failed to remove socket file {} on drop: {err}",
                self.path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试 socket 文件名同时包含日期与 UUID，且扩展名正确。
    ///
    /// - 手段：用固定日期 `20260913` 调用 `generate_path`。
    /// - 判断：文件名必须以 `kb-20260913-` 开头、以 `.sock` 结尾，
    ///   且中间部分的 UUID 为 32 位十六进制字符。
    #[test]
    fn generate_path_embeds_date_and_uuid() {
        let path = generate_path(Path::new("/run/llm_kb"), "20260913");
        let file_name = path.file_name().unwrap().to_str().unwrap();

        assert!(
            file_name.starts_with("kb-20260913-"),
            "实际文件名: {file_name}"
        );
        assert!(file_name.ends_with(".sock"), "实际文件名: {file_name}");

        let uuid_part = file_name
            .trim_start_matches("kb-20260913-")
            .trim_end_matches(".sock");
        assert_eq!(uuid_part.len(), 32, "UUID simple 形式应为 32 字符");
        assert!(
            uuid_part.chars().all(|c| c.is_ascii_hexdigit()),
            "UUID 部分应全部为十六进制字符，实际: {uuid_part}"
        );
    }

    /// 测试连续两次生成的路径不会重复。
    ///
    /// - 手段：同一日期、同一目录下连续生成 128 个路径。
    /// - 判断：把它们放进 `HashSet` 后长度仍为 128，说明 UUID 提供了足够的唯一性；
    ///   这直接支撑「每次启动拼接 UUID」这一设计目标。
    #[test]
    fn generate_path_is_unique_per_call() {
        use std::collections::HashSet;

        let paths: HashSet<PathBuf> = (0..128)
            .map(|_| generate_path(Path::new("/run/llm_kb"), "20260913"))
            .collect();

        assert_eq!(paths.len(), 128, "生成的路径不应重复");
    }

    /// 测试公历换算在若干已知日期上正确。
    ///
    /// - 手段：把若干已知日期换算成「自 1970-01-01 起的天数」后调用 `civil_from_days`。
    /// - 判断：返回的年月日与预期完全一致，覆盖平年、闰年与闰世纪三类情况。
    #[test]
    fn civil_from_days_matches_known_dates() {
        // 1970-01-01 是第 0 天。
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2000-02-29 是闰日（2000 年是闰年，且被 400 整除）。
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        // 2024-02-29 同样是一个闰日。
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        // 2026-09-13（本项目的日期示例）。
        assert_eq!(civil_from_days(20_709), (2026, 9, 13));
    }

    /// 测试 `today_utc_compact` 生成的是 8 位十进制日期，且与系统时钟一致。
    ///
    /// - 手段：调用 `today_utc_compact()` 得到形如 `YYYYMMDD` 的字符串；再用
    ///   `date -u +%Y%m%d` 这一外部参照命令取得同一时刻的 UTC 日期。
    /// - 判断：字符串为 8 位纯数字，且与外部参照逐字节相等。用外部命令而不是
    ///   被测代码自身的换算函数做对照，才能避免自证式的循环断言。
    #[test]
    fn today_utc_compact_matches_date_command() {
        let compact = today_utc_compact();
        assert_eq!(compact.len(), 8, "日期应为 8 位，实际: {compact}");
        assert!(
            compact.chars().all(|c| c.is_ascii_digit()),
            "日期应全部为数字，实际: {compact}"
        );

        let output = std::process::Command::new("date")
            .args(["-u", "+%Y%m%d"])
            .output();

        match output {
            Ok(output) if output.status.success() => {
                let expected = String::from_utf8(output.stdout).expect("date 输出应为 UTF-8");
                let expected = expected.trim();
                assert_eq!(compact, expected, "生成日期应与 `date -u +%Y%m%d` 一致");
            }
            // 极简环境可能没有 `date` 命令；此时只保留格式断言，不让测试失败。
            _ => eprintln!("跳过与 `date` 的比对：本环境无法执行 date 命令"),
        }
    }

    /// 测试删除不存在的 socket 文件不会报错。
    ///
    /// - 手段：对一个确定不存在的路径调用 `remove_socket_file`。
    /// - 判断：返回 `Ok(())`——清理逻辑必须幂等，否则重复启动会失败。
    #[test]
    fn remove_socket_file_is_idempotent() {
        let path = std::env::temp_dir().join("kb-svc-salvo-nonexistent.sock");

        assert!(remove_socket_file(&path).is_ok());
        assert!(remove_socket_file(&path).is_ok());
    }
}
