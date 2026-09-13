//! 用户设置：LLM 服务选项与 API key 的读取、修改与持久化。
//!
//! # 存储位置与格式
//!
//! 设置以 TOML 保存在一个配置文件里，默认路径为
//! `$XDG_CONFIG_HOME/llm_kb/config.toml`（缺省回退 `~/.config/llm_kb/config.toml`）：
//!
//! ```toml
//! # 每个 `[services.<id>]` 是一份可选的 LLM 服务配置。
//! [services.deepseek]
//! provider = "deepseek"
//! model = "deepseek-chat"
//! base_url = "https://api.deepseek.com"
//! api_key = "sk-..."
//! ```
//!
//! 用 `toml_edit` 而不是 `toml` 反序列化，是为了在「界面里改一个 key」时**保留
//! 用户写在配置里的注释与排版**；只有被修改的键会被重写。
//!
//! # 安全提示
//!
//! 按当前阶段的约定，API key 以**明文**保存在配置文件中，界面也不会隐藏它。
//! 这是刻意为之的简化（见 `dev-notes.md` §14）；正式版本需要改为系统密钥链
//! 或加密存储。界面上呈现的 key 会被遮蔽（见 [`LlmServiceConfig::masked`]），
//! 但那只是防肩窥，不构成任何安全保证。

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::error::{KbSvcError, KbSvcResult};

/// 配置文件的默认目录名（位于 `$XDG_CONFIG_HOME` 或 `~/.config` 下）。
const CONFIG_DIR_NAME: &str = "llm_kb";

/// 配置文件名。
const CONFIG_FILE_NAME: &str = "config.toml";

/// 遮蔽后的 API key 呈现形式。
pub const MASKED_KEY: &str = "••••••••";

/// 一份 LLM 服务的配置。
///
/// 字段刻意保持最小：只包含「连到哪个服务、用哪个模型、用什么凭据」。
/// 更多 provider 特化参数（采样、超时、代理等）留到需要时再加。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmServiceConfig {
    /// provider 标识，例如 `deepseek` / `openai` / `ollama`。
    /// `kb_rig_llm` 将来据此选择 rig 的 client 实现。
    pub provider: String,

    /// 模型名，例如 `deepseek-chat`。
    pub model: String,

    /// API base URL；为空表示使用 provider 的默认地址。
    #[serde(default)]
    pub base_url: String,

    /// API key；为空表示尚未配置。
    #[serde(default)]
    pub api_key: String,
}

impl LlmServiceConfig {
    /// 构造一份服务配置。
    pub fn new(
        provider: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            base_url: base_url.into(),
            api_key: api_key.into(),
        }
    }

    /// 是否已经配置了 API key。
    pub fn has_api_key(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    /// 生成一份把 API key 遮蔽掉的副本，用于回传给浏览器。
    pub fn masked(&self) -> Self {
        Self {
            api_key: if self.has_api_key() {
                MASKED_KEY.to_string()
            } else {
                String::new()
            },
            ..self.clone()
        }
    }
}

/// 完整的设置集合。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// 已配置的 LLM 服务，键是用户可见的服务标识（例如 `deepseek`）。
    ///
    /// 用 `BTreeMap` 而不是 `HashMap` 是为了让写回配置文件与界面列表的顺序稳定。
    #[serde(default)]
    pub services: BTreeMap<String, LlmServiceConfig>,
}

impl Settings {
    /// 解析一份 TOML 配置文本。
    pub fn from_toml(text: &str) -> KbSvcResult<Self> {
        let doc = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|err| KbSvcError::Config(format!("TOML 解析失败: {err}")))?;

        let mut services = BTreeMap::new();

        if let Some(table) = doc.get("services").and_then(|item| item.as_table_like()) {
            for (id, item) in table.iter() {
                let entry = item
                    .as_table_like()
                    .ok_or_else(|| KbSvcError::Config(format!("services.{id} 必须是一个表")))?;

                let service = LlmServiceConfig {
                    provider: read_optional_string(entry, id, "provider")?.unwrap_or_default(),
                    model: read_optional_string(entry, id, "model")?.unwrap_or_default(),
                    base_url: read_optional_string(entry, id, "base_url")?.unwrap_or_default(),
                    api_key: read_optional_string(entry, id, "api_key")?.unwrap_or_default(),
                };

                services.insert(id.to_string(), service);
            }
        }

        Ok(Self { services })
    }

    /// 把设置渲染成 TOML 文本（不保留原有注释）。
    ///
    /// 仅在「新建配置文件」时使用；更新已有文件请走 [`SettingsStore::upsert_service`]，
    /// 那条路径会保留注释。
    pub fn to_toml(&self) -> KbSvcResult<String> {
        let mut doc = toml_edit::DocumentMut::new();

        let mut services = toml_edit::Table::new();
        for (id, service) in &self.services {
            services[id] = toml_edit::Item::Table(service_table(service));
        }
        doc["services"] = toml_edit::Item::Table(services);

        Ok(doc.to_string())
    }
}

/// 读取一个可选字符串字段；字段缺失时返回 `None`，类型不对时报错。
fn read_optional_string(
    entry: &dyn toml_edit::TableLike,
    id: &str,
    key: &str,
) -> KbSvcResult<Option<String>> {
    match entry.get(key) {
        None => Ok(None),
        Some(item) => item
            .as_str()
            .map(|value| Some(value.to_string()))
            .ok_or_else(|| KbSvcError::Config(format!("services.{id}.{key} 必须是字符串"))),
    }
}

/// 构造一份服务的 TOML 表。
fn service_table(service: &LlmServiceConfig) -> toml_edit::Table {
    let mut table = toml_edit::Table::new();
    write_service_keys(&mut table, service);
    table
}

/// 把服务的四个字段写进一个已存在的表。
///
/// 逐键赋值而不是整表替换，是为了保留该表上已有的注释与排版——用户的配置文件
/// 里经常写着「这个 key 从哪来」之类的注释，整表替换会把它们全部抹掉。
fn write_service_keys(entry: &mut toml_edit::Table, service: &LlmServiceConfig) {
    entry["provider"] = toml_edit::value(service.provider.clone());
    entry["model"] = toml_edit::value(service.model.clone());
    entry["base_url"] = toml_edit::value(service.base_url.clone());
    entry["api_key"] = toml_edit::value(service.api_key.clone());
}

/// 设置存储：一个配置文件路径背后的读写操作。
///
/// 所有方法都是「读盘 → 改 → 原子写回」的完整流程，因此并发调用是安全的
/// （见 [`SettingsStore::with_lock`]）。
///
/// # 示例
///
/// ```no_run
/// use kb_svc_salvo::settings::{LlmServiceConfig, SettingsStore};
///
/// # async fn demo() -> Result<(), Box<dyn core::error::Error>> {
/// let store = SettingsStore::file("/tmp/llm_kb/config.toml");
/// store.ensure_exists().await?;
///
/// let service = LlmServiceConfig::new("deepseek", "deepseek-chat", "", "sk-demo");
/// store.upsert_service("deepseek", &service).await?;
///
/// let settings = store.load().await?;
/// assert!(settings.services.contains_key("deepseek"));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub enum SettingsStore {
    /// 读写一个真实的 TOML 文件。
    File(PathBuf),

    /// 纯内存存储，仅用于测试，不会触碰文件系统。
    Memory(std::sync::Arc<tokio::sync::RwLock<Settings>>),
}

impl SettingsStore {
    /// 构造一个文件存储。
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::File(path.into())
    }

    /// 构造一个空的内存存储（测试用）。
    pub fn memory() -> Self {
        Self::Memory(std::sync::Arc::new(tokio::sync::RwLock::new(
            Settings::default(),
        )))
    }

    /// 默认配置文件路径。
    ///
    /// 优先 `$XDG_CONFIG_HOME/llm_kb/config.toml`，其次 `~/.config/llm_kb/config.toml`，
    /// 两者都不可用时回退到当前目录下的 `llm_kb/config.toml`。
    pub fn default_config_path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));

        base.join(CONFIG_DIR_NAME).join(CONFIG_FILE_NAME)
    }

    /// 返回底层文件路径（内存存储时返回 `None`）。
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::File(path) => Some(path.as_path()),
            Self::Memory(_) => None,
        }
    }

    /// 确保配置文件存在；不存在时写入一份带注释的模板。
    ///
    /// 已有文件不会被覆盖。
    pub async fn ensure_exists(&self) -> KbSvcResult<()> {
        let Self::File(path) = self else {
            return Ok(());
        };

        if path.exists() {
            return Ok(());
        }

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(KbSvcError::Io)?;
        }

        // 写入一份示例服务，方便用户直接照着改；内容为空不影响启动。
        let template = "\
# llm_kb 用户配置
#
# 每个 [services.<id>] 是一份可选的 LLM 服务配置，<id> 可以随意取，
# 会作为界面上「服务」下拉框里的名字。
#
# 注意：当前阶段的 API key 是明文保存的（见 dev-notes.md §14），
# 请不要把这份文件提交到版本库。

[services.deepseek]
provider = \"deepseek\"
model = \"deepseek-chat\"
base_url = \"https://api.deepseek.com\"
api_key = \"\"
";

        tokio::fs::write(path, template)
            .await
            .map_err(KbSvcError::Io)?;

        Ok(())
    }

    /// 读取全部设置。
    pub async fn load(&self) -> KbSvcResult<Settings> {
        match self {
            Self::Memory(state) => Ok(state.read().await.clone()),
            Self::File(path) => {
                let text = match tokio::fs::read_to_string(path).await {
                    Ok(text) => text,
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                        return Ok(Settings::default());
                    }
                    Err(err) => return Err(KbSvcError::Io(err)),
                };

                Settings::from_toml(&text)
            }
        }
    }

    /// 新增或覆盖一份服务配置，并立即持久化。
    ///
    /// 文件中已存在的注释会被保留（只改动涉及的键）。
    pub async fn upsert_service(&self, id: &str, service: &LlmServiceConfig) -> KbSvcResult<()> {
        if id.trim().is_empty() {
            return Err(KbSvcError::Config("服务标识不能为空".to_string()));
        }

        match self {
            Self::Memory(state) => {
                state
                    .write()
                    .await
                    .services
                    .insert(id.to_string(), service.clone());
                Ok(())
            }
            Self::File(path) => {
                let mut doc = self.load_document(path).await?;

                let services = doc["services"]
                    .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
                    .as_table_mut()
                    .ok_or_else(|| {
                        KbSvcError::Config("services 必须是一个表（table）".to_string())
                    })?;

                match services.get_mut(id).and_then(|item| item.as_table_mut()) {
                    // 服务已存在：只改内部键，保留用户写在这个表上的注释与排版。
                    Some(entry) => write_service_keys(entry, service),
                    // 新服务：整表插入，没有历史排版需要保留。
                    None => services[id] = toml_edit::Item::Table(service_table(service)),
                }

                write_atomically(path, &doc.to_string()).await
            }
        }
    }

    /// 删除一份服务配置。
    ///
    /// 服务不存在时视为成功（幂等）。
    pub async fn remove_service(&self, id: &str) -> KbSvcResult<()> {
        match self {
            Self::Memory(state) => {
                state.write().await.services.remove(id);
                Ok(())
            }
            Self::File(path) => {
                let mut doc = self.load_document(path).await?;

                let Some(services) = doc.get_mut("services").and_then(|item| item.as_table_mut())
                else {
                    return Ok(());
                };

                services.remove(id);

                write_atomically(path, &doc.to_string()).await
            }
        }
    }

    /// 读取配置文件为一个可编辑的 TOML 文档；文件不存在时返回空文档。
    async fn load_document(&self, path: &Path) -> KbSvcResult<toml_edit::DocumentMut> {
        match tokio::fs::read_to_string(path).await {
            Ok(text) => text
                .parse::<toml_edit::DocumentMut>()
                .map_err(|err| KbSvcError::Config(format!("TOML 解析失败: {err}"))),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Ok(toml_edit::DocumentMut::new())
            }
            Err(err) => Err(KbSvcError::Io(err)),
        }
    }
}

/// 原子地把内容写入文件：先写同目录下的临时文件，再 `rename` 覆盖。
///
/// 这样即使写入过程中进程被杀，也不会留下半截配置文件。
async fn write_atomically(path: &Path, content: &str) -> KbSvcResult<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(KbSvcError::Io)?;
    }

    let temp = path.with_extension("toml.tmp");

    tokio::fs::write(&temp, content)
        .await
        .map_err(KbSvcError::Io)?;

    tokio::fs::rename(&temp, path)
        .await
        .map_err(KbSvcError::Io)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试 TOML 解析能得到全部服务及其四个字段。
    ///
    /// - 手段：解析一段包含两个服务的 TOML 文本，其中一个缺少可选的 `base_url`。
    /// - 判断：服务数量为 2；`deepseek` 的四个字段与文本一致；`local` 的
    ///   `base_url` 与 `api_key` 缺省为空字符串。
    #[test]
    fn parse_toml_reads_all_services() {
        let text = r#"
[services.deepseek]
provider = "deepseek"
model = "deepseek-chat"
base_url = "https://api.deepseek.com"
api_key = "sk-1"

[services.local]
provider = "ollama"
model = "qwen2.5"
"#;

        let settings = Settings::from_toml(text).expect("应当解析成功");
        assert_eq!(settings.services.len(), 2);

        let deepseek = settings.services.get("deepseek").expect("应当有 deepseek");
        assert_eq!(deepseek.provider, "deepseek");
        assert_eq!(deepseek.model, "deepseek-chat");
        assert_eq!(deepseek.base_url, "https://api.deepseek.com");
        assert_eq!(deepseek.api_key, "sk-1");
        assert!(deepseek.has_api_key());

        let local = settings.services.get("local").expect("应当有 local");
        assert_eq!(local.base_url, "");
        assert_eq!(local.api_key, "");
        assert!(!local.has_api_key());
    }

    /// 测试字段类型错误会返回配置错误而不是 panic。
    ///
    /// - 手段：把 `api_key` 写成整数而不是字符串。
    /// - 判断：`from_toml` 返回 `Err(KbSvcError::Config)`。
    #[test]
    fn parse_toml_rejects_wrong_type() {
        let text = r#"
[services.bad]
provider = "deepseek"
model = "m"
api_key = 123
"#;

        match Settings::from_toml(text) {
            Err(KbSvcError::Config(_)) => {}
            other => panic!("应当返回配置错误，实际: {other:?}"),
        }
    }

    /// 测试遮蔽只影响 API key，其它字段原样保留。
    ///
    /// - 手段：对一份有 key 和一份无 key 的服务分别调用 `masked()`。
    /// - 判断：有 key 的被替换为 `MASKED_KEY`，无 key 的保持为空；
    ///   `provider` / `model` / `base_url` 三者不变。
    #[test]
    fn masked_hides_only_api_key() {
        let service = LlmServiceConfig::new("deepseek", "deepseek-chat", "https://x", "sk-secret");
        let masked = service.masked();

        assert_eq!(masked.api_key, MASKED_KEY);
        assert_eq!(masked.provider, service.provider);
        assert_eq!(masked.model, service.model);
        assert_eq!(masked.base_url, service.base_url);

        let empty = LlmServiceConfig::new("local", "m", "", "").masked();
        assert_eq!(empty.api_key, "");
    }

    /// 测试 TOML 渲染能被重新解析（往返一致）。
    ///
    /// - 手段：构造一份设置，渲染成 TOML 后再解析回来。
    /// - 判断：两次得到的 `Settings` 完全相等。
    #[test]
    fn to_toml_round_trips() {
        let mut settings = Settings::default();
        settings.services.insert(
            "deepseek".to_string(),
            LlmServiceConfig::new(
                "deepseek",
                "deepseek-chat",
                "https://api.deepseek.com",
                "sk-1",
            ),
        );

        let text = settings.to_toml().expect("应当渲染成功");
        let parsed = Settings::from_toml(&text).expect("应当解析回来");

        assert_eq!(settings, parsed);
    }

    /// 测试文件存储在原文件保留注释的前提下改写单个键。
    ///
    /// - 手段：写出一份带注释的配置，用 `SettingsStore` 覆盖其中一个服务的 `api_key`。
    /// - 判断：重新读盘后 `api_key` 为新值、其它字段不变，且注释仍然存在。
    #[tokio::test]
    async fn file_store_upsert_keeps_comments() {
        let dir = std::env::temp_dir().join(format!("kb-settings-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("应当能创建临时目录");
        let path = dir.join("config.toml");

        std::fs::write(
            &path,
            "# 这是用户的注释\n[services.deepseek]\nprovider = \"deepseek\"\nmodel = \"deepseek-chat\"\nbase_url = \"\"\napi_key = \"\"\n",
        )
        .expect("应当能写入初始配置");

        let store = SettingsStore::file(&path);
        store
            .upsert_service(
                "deepseek",
                &LlmServiceConfig::new("deepseek", "deepseek-reasoner", "", "sk-new"),
            )
            .await
            .expect("覆盖应当成功");

        let text = std::fs::read_to_string(&path).expect("应当能读回配置");
        assert!(text.contains("# 这是用户的注释"), "注释应当被保留:\n{text}");

        let settings = store.load().await.expect("应当能解析");
        let service = settings.services.get("deepseek").expect("应当有 deepseek");
        assert_eq!(service.api_key, "sk-new");
        assert_eq!(service.model, "deepseek-reasoner");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 测试删除服务是幂等的，且不影响其它服务。
    ///
    /// - 手段：写入两个服务，删除其中一个，再重复删除同一个。
    /// - 判断：第一次删除后只剩另一个服务；第二次删除仍返回 `Ok`。
    #[tokio::test]
    async fn file_store_remove_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("kb-settings-rm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("应当能创建临时目录");
        let path = dir.join("config.toml");

        let store = SettingsStore::file(&path);
        store
            .upsert_service("a", &LlmServiceConfig::new("p", "m", "", "k"))
            .await
            .expect("写入 a 应当成功");
        store
            .upsert_service("b", &LlmServiceConfig::new("p", "m", "", "k"))
            .await
            .expect("写入 b 应当成功");

        store.remove_service("a").await.expect("删除应当成功");
        store.remove_service("a").await.expect("重复删除应当成功");

        let settings = store.load().await.expect("应当能解析");
        assert!(!settings.services.contains_key("a"));
        assert!(settings.services.contains_key("b"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 测试内存存储在未显式写入时返回默认设置。
    ///
    /// - 手段：构造内存存储并直接 `load`。
    /// - 判断：服务集合为空，且 `ensure_exists` 是空操作（不 panic、不报错）。
    #[tokio::test]
    async fn memory_store_starts_empty() {
        let store = SettingsStore::memory();
        store.ensure_exists().await.expect("内存存储应当无副作用");

        assert!(store.load().await.expect("应当能读取").services.is_empty());
    }
}
