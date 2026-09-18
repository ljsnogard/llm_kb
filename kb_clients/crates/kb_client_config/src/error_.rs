//! 连接配置的读写失败。

use std::path::PathBuf;

use thiserror::Error;

/// 读、写、解析配置文件时可能出的错。
#[derive(Debug, Error)]
pub enum ConfigError {
    /// 配置文件还不存在。
    ///
    /// 这**不是**异常情况：首次运行时由界面问用户"怎么连"，然后
    /// [`ClientConfig::save`](crate::ClientConfig::save) 生成它。
    #[error("配置文件不存在: {}（首次运行时应当由界面生成）", .path.display())]
    NotFound {
        /// 期望的路径。
        path: PathBuf,
    },

    /// 读文件失败（不是"不存在"）。
    #[error("读取配置文件失败 {}: {source}", .path.display())]
    Read {
        /// 出错的路径。
        path: PathBuf,

        /// 底层 I/O 错误。
        #[source]
        source: std::io::Error,
    },

    /// 写文件失败。
    #[error("写入配置文件失败 {}: {source}", .path.display())]
    Write {
        /// 出错的路径。
        path: PathBuf,

        /// 底层 I/O 错误。
        #[source]
        source: std::io::Error,
    },

    /// TOML 解析失败（含"`kind` 取值不认识"）。
    #[error("配置文件不是合法 TOML: {0}")]
    Parse(#[source] toml::de::Error),

    /// 序列化成 TOML 失败（理论上不会：字段都是基本类型）。
    #[error("配置文件编码失败: {0}")]
    Encode(#[source] toml::ser::Error),

    /// 推导不出配置目录（`HOME` / `XDG_CONFIG_HOME` / `APPDATA` 都没有）。
    #[error("找不到可写的配置目录（HOME / XDG_CONFIG_HOME / APPDATA 都没有给出）")]
    NoConfigDir,

    /// 配置里没有这个连接方式。
    #[error("配置里没有名为 {0:?} 的连接方式")]
    UnknownConnection(String),

    /// 配置里一条连接方式都没有。
    #[error("配置里没有任何连接方式")]
    NoConnections,

    /// 配置文件声明的版本不是本程序认识的那个。
    #[error("配置文件的版本是 {0}，本程序只认识 1（见 CONFIG_VERSION）")]
    UnsupportedVersion(u32),

    /// 连接方式的名字为空。
    #[error("连接方式的名字不能为空")]
    EmptyName,

    /// 两条连接方式用了同一个名字。
    #[error("连接方式名字重复: {0:?}")]
    DuplicateName(String),

    /// 某条连接方式的必填字段为空。
    #[error("连接方式 {connection:?} 的 {field} 不能为空")]
    EmptyField {
        /// 出问题的那条连接方式。
        connection: String,

        /// 字段名（配置文件里的键名）。
        field: &'static str,
    },

    /// `default` 指向的连接方式不存在。
    #[error("default 指向的连接方式不存在: {0:?}")]
    UnknownDefault(String),
}
