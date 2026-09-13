//! # kb_core
//!
//! 知识库主进程的可执行入口。
//!
//! 本 crate 只做三件事：
//!
//! 1. 解析命令行参数；
//! 2. 初始化日志；
//! 3. 调用 [`kb_svc_salvo::launch::launch`]，把参数原样交给它。
//!
//! **其余一切**——配置文件的读取与生成、监听绑定、socket 文件的生成与清理、
//! 信号处理与优雅退出——都由 `kb_svc_salvo` 负责，见
//! `kb_svc/crates/kb_svc_salvo/src/launch.rs` 的模块文档。
//!
//! # 用法
//!
//! ```text
//! kb_core [--config <file>] [--runtime-dir <dir>] [--assets-dir <dir>] [tcp_addr]
//! ```
//!
//! - `tcp_addr`：用户侧 HTTP 监听地址，缺省 `127.0.0.1:8788`；端口写 `0` 表示由系统分配。
//! - `--config`：配置文件路径，缺省 `$XDG_CONFIG_HOME/llm_kb/config.toml`。
//! - `--runtime-dir`：插件通道 socket 的存放目录，缺省 `$XDG_RUNTIME_DIR/llm_kb`。
//! - `--assets-dir`：前端资源覆盖目录（开发期用），缺省使用编译期内嵌资源。
//!
//! 更完整的操作手册见本 crate 的 `README.md`。

use kb_svc_salvo::launch::{LaunchConfig, launch};
use log::warn;

/// 被覆盖的启动参数。
///
/// 字段都是 `Option`：`None` 表示「用 `kb_svc_salvo` 的默认值」，本 crate 不重复
/// 定义这些默认值，避免两处默认值随时间漂移。
#[derive(Debug, Default, PartialEq, Eq)]
struct Args {
    /// 用户侧监听地址。
    tcp_addr: Option<String>,

    /// 配置文件路径。
    config_path: Option<std::path::PathBuf>,

    /// 运行时目录。
    runtime_dir: Option<std::path::PathBuf>,

    /// 前端资源覆盖目录。
    assets_dir: Option<std::path::PathBuf>,
}

/// 解析命令行参数。
///
/// 参数顺序不敏感，未知参数会被忽略并打印警告。
fn parse_args<I>(argv: I) -> Args
where
    I: IntoIterator<Item = String>,
{
    let mut args = Args::default();
    let mut iter = argv.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--config" => match iter.next() {
                Some(value) => args.config_path = Some(value.into()),
                None => warn!("--config 缺少取值，已忽略"),
            },
            "--runtime-dir" => match iter.next() {
                Some(value) => args.runtime_dir = Some(value.into()),
                None => warn!("--runtime-dir 缺少取值，已忽略"),
            },
            "--assets-dir" => match iter.next() {
                Some(value) => args.assets_dir = Some(value.into()),
                None => warn!("--assets-dir 缺少取值，已忽略"),
            },
            other if other.starts_with('-') => warn!("忽略未知参数: {other}"),
            addr => {
                if args.tcp_addr.is_some() {
                    warn!("重复的监听地址参数，已忽略: {addr}");
                } else {
                    args.tcp_addr = Some(addr.to_string());
                }
            }
        }
    }

    args
}

/// 把命令行参数翻译成 `kb_svc_salvo` 的启动配置。
fn to_launch_config(args: Args) -> LaunchConfig {
    let mut config = LaunchConfig::new();

    if let Some(addr) = args.tcp_addr {
        config = config.with_tcp_addr(addr);
    }
    if let Some(path) = args.config_path {
        config = config.with_config_path(path);
    }
    if let Some(dir) = args.runtime_dir {
        config = config.with_runtime_dir(dir);
    }
    if let Some(dir) = args.assets_dir {
        config = config.with_assets_dir(dir);
    }

    config
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let config = to_launch_config(parse_args(std::env::args().skip(1)));

    launch(config).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试参数解析正确识别全部四个选项。
    ///
    /// - 手段：把一组参数以 `String` 迭代器的形式传给 `parse_args`，顺序打乱以验证
    ///   解析与顺序无关。
    /// - 判断：返回的 `Args` 与预期完全相等。
    #[test]
    fn parse_args_reads_all_options() {
        let args = parse_args(
            [
                "--assets-dir",
                "/tmp/assets",
                "127.0.0.1:9999",
                "--config",
                "/tmp/kb.toml",
                "--runtime-dir",
                "/run/kb",
            ]
            .into_iter()
            .map(String::from),
        );

        assert_eq!(
            args,
            Args {
                tcp_addr: Some("127.0.0.1:9999".to_string()),
                config_path: Some("/tmp/kb.toml".into()),
                runtime_dir: Some("/run/kb".into()),
                assets_dir: Some("/tmp/assets".into()),
            }
        );
    }

    /// 测试缺省参数时四个字段都为空，由 `kb_svc_salvo` 补默认值。
    ///
    /// - 手段：传入空参数列表。
    /// - 判断：`Args` 等于 `Default`。
    #[test]
    fn parse_args_defaults_to_empty() {
        assert_eq!(parse_args(std::iter::empty()), Args::default());
    }

    /// 测试未知参数与重复地址不会破坏解析结果。
    ///
    /// - 手段：传入一个未知开关、两次重复的监听地址，以及缺少取值的 `--config`。
    /// - 判断：第一个监听地址被保留，`config_path` 保持为 `None`，解析过程不 panic。
    #[test]
    fn parse_args_tolerates_unknown_and_duplicate() {
        let args = parse_args(
            ["--nope", "127.0.0.1:1", "127.0.0.1:2", "--config"]
                .into_iter()
                .map(String::from),
        );

        assert_eq!(
            args,
            Args {
                tcp_addr: Some("127.0.0.1:1".to_string()),
                config_path: None,
                runtime_dir: None,
                assets_dir: None,
            }
        );
    }

    /// 测试只有被显式指定的参数才会进入 `LaunchConfig`。
    ///
    /// - 手段：只设置 `runtime_dir`，其余字段保持缺省，然后读取 `LaunchConfig` 的访问器。
    /// - 判断：只有运行时目录被设置，另外三项仍为 `None`——默认值由 `kb_svc_salvo` 决定，
    ///   本 crate 不参与，避免两处默认值漂移。
    #[test]
    fn to_launch_config_only_carries_given_options() {
        let config = to_launch_config(Args {
            runtime_dir: Some("/tmp/run".into()),
            ..Args::default()
        });

        assert_eq!(config.tcp_addr(), None);
        assert_eq!(config.config_path(), None);
        assert_eq!(config.runtime_dir(), Some(std::path::Path::new("/tmp/run")));
        assert_eq!(config.assets_dir(), None);
    }
}
