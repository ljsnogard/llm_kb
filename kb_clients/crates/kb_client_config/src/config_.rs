//! 连接配置的内容：`kb_admin_desktop` 怎么找到 / 启动 `kb_core`。
//!
//! 格式是 **TOML**（给人手改的：有注释、嵌套不吵）；这份文件**不是**协议，
//! 只在客户端自己这一侧使用，因此可以用 serde 的内部标签
//! （`kind = "tcp"`）与 `skip_serializing_if`——那两条禁令是给走 postcard 的
//! 协议类型准备的，见 `abs_kb_svc_v1_desktop` 的模块文档。
//!
//! # 三种连接方式
//!
//! ```toml
//! version = 1
//! default = "本机"
//!
//! [[connections]]
//! name = "本机"
//! kind = "local-launch"          # 起一个本机 kb_core 并连它的 IPC
//! kb_core = "/usr/local/bin/kb-core"
//! runtime_dir = "/run/user/1000/llm_kb"
//! storage_dir = "/run/user/1000/llm_kb/storage"
//! handshake_timeout_millis = 10000
//!
//! [[connections]]
//! name = "已在本机跑着的"
//! kind = "local-attach"          # 只连已经在跑的 kb_core
//! runtime_dir = "/run/user/1000/llm_kb"
//! connect_timeout_millis = 5000
//!
//! [[connections]]
//! name = "实验室"
//! kind = "tcp"                   # 经 kb_core_rproxy 走 TCP
//! address = "192.168.1.5:8788"
//! connect_timeout_millis = 5000
//! request_timeout_millis = 15000
//! ```

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error_::ConfigError;

/// 这份配置的格式版本。
///
/// 与协议版本无关：它只关乎这个文件怎么写。将来若改了字段形状，读的时候就能
/// 明确拒绝"看不懂的版本"，而不是把旧字段当成新字段用。
pub const CONFIG_VERSION: u32 = 1;

/// 缺省的"等 `kb_core` 公布端点"的时长（`local-launch`）。
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// 缺省的"连上去"的时长（`local-attach` 与 `tcp`）。
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// 缺省的单次请求时长。
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// 连接方式的种类。
///
/// 单独列一个枚举，是为了让界面能列出"有哪些可选方式"而不必解析字段；
/// TOML 里的取值是 [`ConnectionKind::as_str`] 给出的字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConnectionKind {
    /// 启动一个本机 `kb_core` 子进程，再连它的 IPC。
    LocalLaunch,

    /// 只连已经在跑的本机 `kb_core`（扫运行时目录里的端点名字文件）。
    LocalAttach,

    /// 经 `kb_core_rproxy` 走 TCP。
    Tcp,
}

impl ConnectionKind {
    /// 配置文件里的取值（也是界面上的标识）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalLaunch => "local-launch",
            Self::LocalAttach => "local-attach",
            Self::Tcp => "tcp",
        }
    }

    /// 从配置文件里的取值解析；不认识时返回 `None`。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "local-launch" => Some(Self::LocalLaunch),
            "local-attach" => Some(Self::LocalAttach),
            "tcp" => Some(Self::Tcp),
            _ => None,
        }
    }

    /// 界面上给人看的一句话说明。
    pub fn description(self) -> &'static str {
        match self {
            Self::LocalLaunch => "启动一个本机的 kb_core，并连上它的 IPC",
            Self::LocalAttach => "连接已经在跑的本机 kb_core",
            Self::Tcp => "经 kb_core_rproxy 连接远程 kb_core（无鉴权，仅受信网络）",
        }
    }
}

/// 一条连接方式。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Connection {
    /// 起一个本机 `kb_core` 子进程，再连它。
    LocalLaunch {
        /// 界面上显示的名字，也是 `default` 引用的键。
        name: String,

        /// `kb_core` 可执行文件。
        kb_core: PathBuf,

        /// 运行时目录（IPC 端点名字文件放在这里）。
        runtime_dir: PathBuf,

        /// 知识库数据目录。
        storage_dir: PathBuf,

        /// 等 `kb_core` 公布端点的时限；缺省 [`DEFAULT_HANDSHAKE_TIMEOUT`]。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        handshake_timeout_millis: Option<u64>,
    },

    /// 只连已经在跑的本机 `kb_core`。
    LocalAttach {
        /// 界面上显示的名字。
        name: String,

        /// 运行时目录。
        runtime_dir: PathBuf,

        /// 连上去的时限；缺省 [`DEFAULT_CONNECT_TIMEOUT`]。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        connect_timeout_millis: Option<u64>,
    },

    /// 经 `kb_core_rproxy` 走 TCP。
    Tcp {
        /// 界面上显示的名字。
        name: String,

        /// 网关地址，`主机:端口`。
        address: String,

        /// 建立 TCP 连接的时限；缺省 [`DEFAULT_CONNECT_TIMEOUT`]。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        connect_timeout_millis: Option<u64>,

        /// 单次请求的时限；缺省 [`DEFAULT_REQUEST_TIMEOUT`]。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_timeout_millis: Option<u64>,
    },
}

impl Connection {
    /// 造一条"启动本机 `kb_core`"的连接，时限用缺省值。
    pub fn local_launch(
        name: impl Into<String>,
        kb_core: impl Into<PathBuf>,
        runtime_dir: impl Into<PathBuf>,
        storage_dir: impl Into<PathBuf>,
    ) -> Self {
        Self::LocalLaunch {
            name: name.into(),
            kb_core: kb_core.into(),
            runtime_dir: runtime_dir.into(),
            storage_dir: storage_dir.into(),
            handshake_timeout_millis: None,
        }
    }

    /// 造一条"附着到已在跑的 `kb_core`"的连接，时限用缺省值。
    pub fn local_attach(name: impl Into<String>, runtime_dir: impl Into<PathBuf>) -> Self {
        Self::LocalAttach {
            name: name.into(),
            runtime_dir: runtime_dir.into(),
            connect_timeout_millis: None,
        }
    }

    /// 造一条 TCP 连接，时限用缺省值。
    pub fn tcp(name: impl Into<String>, address: impl Into<String>) -> Self {
        Self::Tcp {
            name: name.into(),
            address: address.into(),
            connect_timeout_millis: None,
            request_timeout_millis: None,
        }
    }

    /// 界面上显示的名字。
    pub fn name(&self) -> &str {
        match self {
            Self::LocalLaunch { name, .. }
            | Self::LocalAttach { name, .. }
            | Self::Tcp { name, .. } => name,
        }
    }

    /// 种类。
    pub fn kind(&self) -> ConnectionKind {
        match self {
            Self::LocalLaunch { .. } => ConnectionKind::LocalLaunch,
            Self::LocalAttach { .. } => ConnectionKind::LocalAttach,
            Self::Tcp { .. } => ConnectionKind::Tcp,
        }
    }

    /// 等 `kb_core` 公布端点的时限（只有 `local-launch` 用得上）。
    pub fn handshake_timeout(&self) -> Duration {
        match self {
            Self::LocalLaunch {
                handshake_timeout_millis,
                ..
            } => millis_or_(*handshake_timeout_millis, DEFAULT_HANDSHAKE_TIMEOUT),
            _ => DEFAULT_HANDSHAKE_TIMEOUT,
        }
    }

    /// 建立连接的时限。
    pub fn connect_timeout(&self) -> Duration {
        match self {
            Self::LocalLaunch { .. } => DEFAULT_CONNECT_TIMEOUT,
            Self::LocalAttach {
                connect_timeout_millis,
                ..
            }
            | Self::Tcp {
                connect_timeout_millis,
                ..
            } => millis_or_(*connect_timeout_millis, DEFAULT_CONNECT_TIMEOUT),
        }
    }

    /// 单次请求的时限。
    pub fn request_timeout(&self) -> Duration {
        match self {
            Self::Tcp {
                request_timeout_millis,
                ..
            } => millis_or_(*request_timeout_millis, DEFAULT_REQUEST_TIMEOUT),
            _ => DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// 检查这一条自身是否可用（名字非空、必填字段非空）。
    fn validate_(&self) -> Result<(), ConfigError> {
        if self.name().trim().is_empty() {
            return Err(ConfigError::EmptyName);
        }

        let empty = |field: &'static str| ConfigError::EmptyField {
            connection: self.name().to_string(),
            field,
        };

        match self {
            Self::LocalLaunch {
                kb_core,
                runtime_dir,
                storage_dir,
                ..
            } => {
                if kb_core.as_os_str().is_empty() {
                    return Err(empty("kb_core"));
                }
                if runtime_dir.as_os_str().is_empty() {
                    return Err(empty("runtime_dir"));
                }
                if storage_dir.as_os_str().is_empty() {
                    return Err(empty("storage_dir"));
                }
            }
            Self::LocalAttach { runtime_dir, .. } => {
                if runtime_dir.as_os_str().is_empty() {
                    return Err(empty("runtime_dir"));
                }
            }
            Self::Tcp { address, .. } => {
                if address.trim().is_empty() {
                    return Err(empty("address"));
                }
            }
        }

        Ok(())
    }
}

/// 整个配置文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientConfig {
    /// 格式版本（[`CONFIG_VERSION`]）。
    pub version: u32,

    /// 缺省用哪一条连接方式；缺省时用第一条。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    /// 若干条连接方式。
    #[serde(default)]
    pub connections: Vec<Connection>,
}

impl ClientConfig {
    /// 造一份只有一条连接方式的配置。
    pub fn with_single(connection: Connection) -> Self {
        Self {
            version: CONFIG_VERSION,
            default: Some(connection.name().to_string()),
            connections: vec![connection],
        }
    }

    /// 从 TOML 文本解析。
    ///
    /// # Errors
    ///
    /// 语法/字段不合法 → [`ConfigError::Parse`]；版本不认识、或校验不过
    /// （空名字、重名、`default` 指向不存在的项）→ 相应的 [`ConfigError`]。
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(text).map_err(ConfigError::Parse)?;
        config.validate()?;
        Ok(config)
    }

    /// 序列化成 TOML 文本。
    ///
    /// # Errors
    ///
    /// 字段都是基本类型，正常不会失败；失败时返回 [`ConfigError::Encode`]。
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        toml::to_string_pretty(self).map_err(ConfigError::Encode)
    }

    /// 读配置文件。
    ///
    /// 文件不存在时返回 [`ConfigError::NotFound`]——**这不是异常**：首次运行时
    /// 由界面问用户"怎么连"，再调用 [`ClientConfig::save`] 生成它。
    ///
    /// # Errors
    ///
    /// 见 [`ConfigError`]。
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Err(ConfigError::NotFound {
                    path: path.to_path_buf(),
                });
            }
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };

        Self::from_toml(&text)
    }

    /// 原子地写配置文件：先写同目录下的 `.tmp`，再 `rename`。
    ///
    /// 目录不存在会先建出来。`rename` 保证读到的要么是旧内容、要么是新内容，
    /// 不会是被写了一半的 TOML——与 `kb_core` 存储层的约定一致。
    ///
    /// # Errors
    ///
    /// 校验不过 → 相应的 [`ConfigError`]；建目录/写/改名失败 →
    /// [`ConfigError::Write`]。
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        self.validate()?;
        let text = self.to_toml()?;

        let write_error = |source: std::io::Error| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        };

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(write_error)?;
        }

        let temp = path.with_extension("toml.tmp");
        std::fs::write(&temp, text).map_err(write_error)?;
        std::fs::rename(&temp, path).map_err(write_error)?;

        log::info!("连接配置已写入 {}", path.display());
        Ok(())
    }

    /// 检查整份配置是否自洽。
    ///
    /// # Errors
    ///
    /// - 版本不是 [`CONFIG_VERSION`]；
    /// - 一条连接方式都没有（[`ConfigError::NoConnections`]）；
    /// - 名字为空 / 重名 / 必填字段为空；
    /// - `default` 指向不存在的连接方式。
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.version != CONFIG_VERSION {
            return Err(ConfigError::UnsupportedVersion(self.version));
        }
        if self.connections.is_empty() {
            return Err(ConfigError::NoConnections);
        }

        let mut names = BTreeSet::new();
        for connection in &self.connections {
            connection.validate_()?;
            if !names.insert(connection.name().to_string()) {
                return Err(ConfigError::DuplicateName(connection.name().to_string()));
            }
        }

        if let Some(default) = &self.default
            && !names.contains(default)
        {
            return Err(ConfigError::UnknownDefault(default.clone()));
        }

        Ok(())
    }

    /// 按名字取一条连接方式。
    ///
    /// # Errors
    ///
    /// 没有这个名字时返回 [`ConfigError::UnknownConnection`]。
    pub fn connection(&self, name: &str) -> Result<&Connection, ConfigError> {
        self.connections
            .iter()
            .find(|connection| connection.name() == name)
            .ok_or_else(|| ConfigError::UnknownConnection(name.to_string()))
    }

    /// 缺省该用哪一条：`default` 指定的那条；没指定就用第一条。
    ///
    /// # Errors
    ///
    /// 一条都没有时返回 [`ConfigError::NoConnections`]。
    pub fn default_connection(&self) -> Result<&Connection, ConfigError> {
        match &self.default {
            Some(name) => self.connection(name),
            None => self.connections.first().ok_or(ConfigError::NoConnections),
        }
    }
}

/// 把"毫秒数"翻成时长；`None` 或 `0` 用缺省值。
fn millis_or_(millis: Option<u64>, fallback: Duration) -> Duration {
    match millis {
        Some(0) | None => fallback,
        Some(millis) => Duration::from_millis(millis),
    }
}

#[cfg(test)]
mod tests_ {
    use super::*;

    /// 组装一份覆盖三种连接方式的配置。
    fn sample_() -> ClientConfig {
        ClientConfig {
            version: CONFIG_VERSION,
            default: Some("本机".to_string()),
            connections: vec![
                Connection::local_launch(
                    "本机",
                    "/usr/local/bin/kb-core",
                    "/run/kb",
                    "/run/kb/storage",
                ),
                Connection::local_attach("附着", "/run/kb"),
                Connection::tcp("实验室", "192.168.1.5:8788"),
            ],
        }
    }

    /// 测试 TOML 往返：三种 `kind` 都能写出去、读回来，取值一字不差。
    ///
    /// - 手段：把覆盖三种连接方式的配置序列化成 TOML，再解析回来。
    /// - 判断：往返结果与原值相等；`kind` 写成 kebab-case；缺省时限不写进文件
    ///   （`handshake_timeout_millis` 之类只在显式给过时才出现），这样手写的配置
    ///   文件干净，而"缺省值是什么"由常量单独决定。
    #[test]
    fn config_round_trips_through_toml_() {
        let config = sample_();
        let text = config.to_toml().expect("应当能序列化");

        assert!(
            text.contains("kind = \"local-launch\""),
            "实际 TOML:\n{text}"
        );
        assert!(
            text.contains("kind = \"local-attach\""),
            "实际 TOML:\n{text}"
        );
        assert!(text.contains("kind = \"tcp\""), "实际 TOML:\n{text}");
        assert!(
            !text.contains("timeout_millis"),
            "没显式给过的时限不该写进文件:\n{text}"
        );

        let parsed = ClientConfig::from_toml(&text).expect("应当能解析");
        assert_eq!(parsed, config);
    }

    /// 测试显式时限会被保留，且认得出的取值换算正确。
    ///
    /// - 手段：造一条给了 `handshake_timeout_millis = 1234` 的 `local-launch`，
    ///   以及一条给了 `request_timeout_millis = 4321` 的 `tcp`，往返一次并与
    ///   缺省值对比。
    /// - 判断：往返相等；`handshake_timeout()` 是 1234ms，`request_timeout()` 是
    ///   4321ms；没给时限的 `local-attach` 用 [`DEFAULT_CONNECT_TIMEOUT`]。
    #[test]
    fn explicit_timeouts_survive_and_defaults_apply_() {
        let mut config = sample_();
        config.connections[0] = Connection::LocalLaunch {
            name: "本机".to_string(),
            kb_core: PathBuf::from("/usr/local/bin/kb-core"),
            runtime_dir: PathBuf::from("/run/kb"),
            storage_dir: PathBuf::from("/run/kb/storage"),
            handshake_timeout_millis: Some(1234),
        };
        config.connections[2] = Connection::Tcp {
            name: "实验室".to_string(),
            address: "192.168.1.5:8788".to_string(),
            connect_timeout_millis: None,
            request_timeout_millis: Some(4321),
        };

        let text = config.to_toml().expect("应当能序列化");
        let parsed = ClientConfig::from_toml(&text).expect("应当能解析");
        assert_eq!(parsed, config);

        assert_eq!(
            parsed.connections[0].handshake_timeout(),
            Duration::from_millis(1234)
        );
        assert_eq!(
            parsed.connections[2].request_timeout(),
            Duration::from_millis(4321)
        );
        assert_eq!(
            parsed.connections[1].connect_timeout(),
            DEFAULT_CONNECT_TIMEOUT
        );
    }

    /// 测试坏配置会被明确拒绝，而不是被"尽力而为"地接受。
    ///
    /// - 手段：分别构造"版本不认识"、"一条连接都没有"、"名字为空"、"名字重复"、
    ///   "`default` 指向不存在"、"tcp 地址为空"、"`kind` 取值不认识"。
    /// - 判断：前六种各自得到对应的 [`ConfigError`] 变体；最后一种在解析阶段就
    ///   失败（[`ConfigError::Parse`]）——手改配置写错 `kind` 必须立刻报出来。
    #[test]
    fn bad_configs_are_rejected_() {
        let mut wrong_version = sample_();
        wrong_version.version = 99;
        assert!(matches!(
            wrong_version.validate(),
            Err(ConfigError::UnsupportedVersion(99))
        ));

        let empty = ClientConfig {
            version: CONFIG_VERSION,
            default: None,
            connections: Vec::new(),
        };
        assert!(matches!(empty.validate(), Err(ConfigError::NoConnections)));

        let mut blank_name = sample_();
        blank_name.connections[1] = Connection::local_attach("   ", "/run/kb");
        assert!(matches!(blank_name.validate(), Err(ConfigError::EmptyName)));

        let mut duplicate = sample_();
        duplicate.connections[1] = Connection::local_attach("本机", "/run/kb");
        assert!(matches!(
            duplicate.validate(),
            Err(ConfigError::DuplicateName(name)) if name == "本机"
        ));

        let mut bad_default = sample_();
        bad_default.default = Some("不存在".to_string());
        assert!(matches!(
            bad_default.validate(),
            Err(ConfigError::UnknownDefault(name)) if name == "不存在"
        ));

        let mut empty_address = sample_();
        empty_address.connections[2] = Connection::tcp("实验室", "  ");
        assert!(matches!(
            empty_address.validate(),
            Err(ConfigError::EmptyField {
                field: "address",
                ..
            })
        ));

        let text = r#"
version = 1
[[connections]]
name = "怪东西"
kind = "carrier-pigeon"
"#;
        assert!(matches!(
            ClientConfig::from_toml(text),
            Err(ConfigError::Parse(_))
        ));
    }

    /// 测试"没有配置文件"与"配置坏了"是两件事。
    ///
    /// - 手段：在一个空临时目录里 `load` 一个不存在的路径；再写一份坏 TOML 后
    ///   `load` 它。
    /// - 判断：前者是 [`ConfigError::NotFound`]（界面据此走"首次运行"分支），
    ///   后者是 [`ConfigError::Parse`]（界面应当让用户修，而不是静默覆盖）。
    #[test]
    fn missing_file_is_not_the_same_as_broken_file_() {
        let guard = tempfile::tempdir().expect("临时目录");
        let path = guard.path().join("config.toml");

        assert!(matches!(
            ClientConfig::load(&path),
            Err(ConfigError::NotFound { .. })
        ));

        std::fs::write(&path, "这不是 TOML").expect("应当能写");
        assert!(matches!(
            ClientConfig::load(&path),
            Err(ConfigError::Parse(_))
        ));
    }

    /// 测试保存是原子写（先临时文件再改名），且能原样读回。
    ///
    /// - 手段：把样例配置 `save` 到临时目录下**还不存在**的子目录里，再 `load`。
    /// - 判断：返回 `Ok`；读回来的配置与原值相等；目录被自动建出来；过程中没有
    ///   留下 `.tmp` 残留。
    #[test]
    fn save_is_atomic_and_round_trips_() {
        let guard = tempfile::tempdir().expect("临时目录");
        let path = guard.path().join("nested").join("config.toml");
        let config = sample_();

        config.save(&path).expect("应当能保存");

        assert!(path.is_file(), "应当写出配置文件");
        assert!(
            !path.with_extension("toml.tmp").exists(),
            "不应当留下临时文件"
        );
        assert_eq!(ClientConfig::load(&path).expect("应当能读回"), config);
    }

    /// 测试缺省连接的选择规则：`default` 优先，没有就用第一条。
    ///
    /// - 手段：分别用 `default = Some("实验室")`、`default = None` 调
    ///   `default_connection()`，再取一个不存在的名字。
    /// - 判断：前者给出"实验室"，中者给出第一条"本机"，后者返回
    ///   [`ConfigError::UnknownConnection`]。
    #[test]
    fn default_connection_selection_() {
        let mut config = sample_();

        config.default = Some("实验室".to_string());
        assert_eq!(
            config.default_connection().expect("应当能取到").name(),
            "实验室"
        );

        config.default = None;
        assert_eq!(
            config.default_connection().expect("应当能取到").name(),
            "本机"
        );

        assert!(matches!(
            config.connection("没有这个"),
            Err(ConfigError::UnknownConnection(name)) if name == "没有这个"
        ));
    }

    /// 测试三种 `kind` 的字符串与解析是对称的。
    ///
    /// - 手段：对 [`ConnectionKind`] 的三个取值做 `as_str` → `from_str` 往返，
    ///   再喂一个不认识的字符串。
    /// - 判断：往返得到同一个值；不认识的返回 `None`。这条钉住"界面下拉框里的取值"
    ///   与"配置文件里写的取值"是同一套。
    #[test]
    fn kind_strings_are_symmetric_() {
        for kind in [
            ConnectionKind::LocalLaunch,
            ConnectionKind::LocalAttach,
            ConnectionKind::Tcp,
        ] {
            assert_eq!(ConnectionKind::parse(kind.as_str()), Some(kind));
            assert!(!kind.description().is_empty());
        }
        assert_eq!(ConnectionKind::parse("carrier-pigeon"), None);
    }
}
