//! 配置文件的**位置**：显式覆盖 → 平台约定目录。
//!
//! 客户端自己决定配置放哪，`kb_core` 不知道也不需要知道这份文件——它只描述
//! "客户端要怎么找到 / 启动 `kb_core`"。

use std::path::{Path, PathBuf};

/// 显式指定配置文件的**环境变量**。
///
/// 设置之后优先于平台约定目录；开发、排错、或者同时维护多份配置时用。
pub const CONFIG_PATH_ENV: &str = "KB_ADMIN_DESKTOP_CONFIG";

/// 应用在配置目录下的子目录名。
pub const APP_DIR: &str = "kb_admin_desktop";

/// 配置文件名。
pub const CONFIG_FILE: &str = "config.toml";

/// 配置文件的路径：先看 [`CONFIG_PATH_ENV`]，再按平台约定推导。
///
/// 返回 `None` 表示既没有显式覆盖、也推导不出平台目录（极少见，通常是环境变量
/// 被清空了）。这种情况下界面应当让用户直接指定一个路径。
pub fn config_path() -> Option<PathBuf> {
    config_path_from_(
        std::env::var_os(CONFIG_PATH_ENV).map(PathBuf::from),
        default_config_path(),
    )
}

/// 平台约定目录下的配置路径。
///
/// | 平台 | 位置 |
/// | :--- | :--- |
/// | Linux 等 | `$XDG_CONFIG_HOME/kb_admin_desktop/config.toml`，缺省 `~/.config/...` |
/// | macOS | `~/Library/Application Support/kb_admin_desktop/config.toml` |
/// | Windows | `%APPDATA%\kb_admin_desktop\config.toml` |
pub fn default_config_path() -> Option<PathBuf> {
    config_dir_().map(|dir| dir.join(APP_DIR).join(CONFIG_FILE))
}

/// `kb_core` 的**缺省运行时目录**（IPC 端点名字文件放在这里）。
///
/// 与 `kb_core` 自己的推导保持一致：`$XDG_RUNTIME_DIR/llm_kb`，没有
/// `XDG_RUNTIME_DIR` 时退到系统临时目录。
///
/// 单独放在这里，是为了让"首次运行"的界面能把它作为输入框的缺省值；
/// `kb_core` 启动时若没显式给 `--runtime-dir`，用的就是这个规则。
pub fn default_runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(std::env::temp_dir)
        .join("llm_kb")
}

/// `kb_core` 的**缺省存储目录**：运行时目录下的 `storage`。
pub fn default_storage_dir(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("storage")
}

/// 显式覆盖（非空才算数）优先于平台推导。
///
/// 做成纯函数只是为了可测：读环境变量的那半边在 [`config_path`] 里。
fn config_path_from_(explicit: Option<PathBuf>, platform: Option<PathBuf>) -> Option<PathBuf> {
    match explicit {
        Some(path) if !path.as_os_str().is_empty() => Some(path),
        _ => platform,
    }
}

/// 平台配置目录（不含应用子目录）。
fn config_dir_() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        non_empty_env_("APPDATA")
    }

    #[cfg(target_os = "macos")]
    {
        home_dir_().map(|home| home.join("Library").join("Application Support"))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        non_empty_env_("XDG_CONFIG_HOME").or_else(|| home_dir_().map(|home| home.join(".config")))
    }
}

/// 读一个"非空才算数"的环境变量。
#[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
fn non_empty_env_(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// 用户主目录。
#[cfg(any(target_os = "macos", all(unix, not(target_os = "macos"))))]
fn home_dir_() -> Option<PathBuf> {
    non_empty_env_("HOME")
}

#[cfg(test)]
mod tests_ {
    use super::*;

    /// 测试缺省运行时目录跟着 `XDG_RUNTIME_DIR` 走，没有它时退到临时目录。
    ///
    /// - 手段：直接调用 `default_runtime_dir()`，与同一套环境变量手工推导的结果对比。
    /// - 判断：结果等于 `<XDG_RUNTIME_DIR 或 temp_dir>/llm_kb`；存储目录是它下面的
    ///   `storage`。这条同时钉住"客户端填入的缺省值"与 `kb_core` 自己的默认规则一致。
    #[test]
    fn default_runtime_dir_follows_xdg_() {
        let expected_root = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(std::env::temp_dir);

        let runtime = default_runtime_dir();
        assert_eq!(runtime, expected_root.join("llm_kb"));
        assert_eq!(default_storage_dir(&runtime), runtime.join("storage"));
    }

    /// 测试"显式覆盖优先、空覆盖不算数"这条路径规则。
    ///
    /// - 手段：直接调用纯函数 `config_path_from_`，分别给 显式+平台、空显式+平台、
    ///   无显式+平台、两者都无 四种输入。
    /// - 判断：非空的显式路径原样胜出；空路径与 `None` 都退回平台路径；
    ///   都没有时返回 `None`（界面据此让用户直接填路径）。
    #[test]
    fn explicit_override_wins_over_platform_dir_() {
        let explicit = PathBuf::from("/tmp/kb-admin-desktop.toml");
        let platform = PathBuf::from("/home/me/.config/kb_admin_desktop/config.toml");

        assert_eq!(
            config_path_from_(Some(explicit.clone()), Some(platform.clone())),
            Some(explicit)
        );
        assert_eq!(
            config_path_from_(Some(PathBuf::new()), Some(platform.clone())),
            Some(platform.clone())
        );
        assert_eq!(
            config_path_from_(None, Some(platform.clone())),
            Some(platform)
        );
        assert_eq!(config_path_from_(None, None), None);
    }
}
