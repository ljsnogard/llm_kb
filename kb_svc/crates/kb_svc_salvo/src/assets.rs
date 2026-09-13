//! 前端静态资源：文件的组织、内嵌与覆盖。
//!
//! # 目录约定
//!
//! 所有前端资源都放在本 crate 的 `src/web/assets/` 下：
//!
//! ```text
//! kb_svc/crates/kb_svc_salvo/src/web/assets/
//!   index.html
//!   app.css
//!   app.js
//! ```
//!
//! # 内嵌与开发期覆盖
//!
//! 默认用 `include_str!` 把资源编译进二进制，因此发布版本是单文件、零外部依赖的。
//! 调试前端时不想每次重新编译，可以给 `kb_core` 传 `--assets-dir <dir>`：
//! 该目录下的同名文件会优先返回，缺文件时回退到内嵌版本。
//!
//! 覆盖目录只在启动时解析一次，保存在 [`AssetSettings`] 里，通过 Salvo 的
//! `affix_state` 注入到 handler 的 `Depot` 中。

use std::path::{Path, PathBuf};

/// 内嵌的首页。
pub const INDEX_HTML: &str = include_str!("web/assets/index.html");

/// 内嵌的样式表。
pub const APP_CSS: &str = include_str!("web/assets/app.css");

/// 内嵌的前端脚本。
pub const APP_JS: &str = include_str!("web/assets/app.js");

/// 一张静态资源的描述。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    /// 磁盘上的相对路径（相对资源目录），例如 `app.js`。
    pub file: &'static str,

    /// 内嵌内容。
    pub embedded: &'static str,
}

/// 首页的资源描述。
pub const INDEX_ASSET: Asset = Asset {
    file: "index.html",
    embedded: INDEX_HTML,
};

/// 除首页外、全部需要对外提供的静态资源。
pub const ASSETS: &[Asset] = &[
    Asset {
        file: "app.css",
        embedded: APP_CSS,
    },
    Asset {
        file: "app.js",
        embedded: APP_JS,
    },
];

/// 静态资源相关的运行时设置。
#[derive(Debug, Clone, Default)]
pub struct AssetSettings {
    /// 开发期覆盖目录；`None` 表示只用内嵌资源。
    override_dir: Option<PathBuf>,
}

impl AssetSettings {
    /// 指定一个覆盖目录。
    pub fn with_override_dir(dir: impl Into<PathBuf>) -> Self {
        Self {
            override_dir: Some(dir.into()),
        }
    }

    /// 返回覆盖目录。
    pub fn override_dir(&self) -> Option<&Path> {
        self.override_dir.as_deref()
    }

    /// 解析某个资源的最终内容：优先读覆盖目录，读不到则回退到内嵌版本。
    pub async fn resolve(&self, asset: &Asset) -> String {
        if let Some(dir) = &self.override_dir {
            let path = dir.join(asset.file);
            match tokio::fs::read_to_string(&path).await {
                Ok(text) => {
                    log::debug!("使用覆盖资源 {} ← {}", asset.file, path.display());
                    return text;
                }
                Err(err) => {
                    log::debug!(
                        "覆盖资源 {} 不可用（{err}），回退到内嵌版本",
                        path.display()
                    );
                }
            }
        }

        asset.embedded.to_string()
    }
}
