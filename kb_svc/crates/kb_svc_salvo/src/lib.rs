//! # kb_svc_salvo
//!
//! 知识库服务的 Salvo 实现：对浏览器的聊天界面 + 对插件的 UDS 通道。
//!
//! 本 crate **只是库**，不提供二进制目标；进程的启动与编排由 `kb_core` 负责。
//!
//! # 模块地图
//!
//! | 模块 | 职责 |
//! | :--- | :--- |
//! | [`launch`] | 启动编排：配置加载、监听绑定、优雅退出与资源清理（`kb_core` 的唯一入口） |
//! | [`server`] | 组装路由表、绑定 TCP + Unix socket 双监听器、注入共享状态 |
//! | [`web`] | 浏览器侧路由：静态资源、设置 API、聊天 WebSocket |
//! | [`web_ws`] | 浏览器连接的事件循环 |
//! | [`plugin`] | 插件侧路由：`/ws/plugin` 双向转发 |
//! | [`plugin_socket`] | 插件通道 socket 的路径生成、权限与清理 |
//! | [`hub`] | 会话中枢：请求转发、turn 状态、事件广播 |
//! | [`wire`] | 两段线协议的 JSON 类型 |
//! | [`settings`] | LLM 服务选项与 API key 的读写与持久化 |
//! | [`assets`] | 前端资源的内嵌与开发期覆盖 |
//! | [`poc`] | **临时**：PoC 证据留存，见模块文档 |
//!
//! # 最小用法
//!
//! ```no_run
//! use std::sync::Arc;
//!
//! use kb_svc_salvo::{hub::AppState, server::{ServerConfig, bind}, settings::SettingsStore};
//!
//! # async fn demo() -> Result<(), Box<dyn core::error::Error>> {
//! let store = SettingsStore::file("/tmp/llm_kb/config.toml");
//! store.ensure_exists().await?;
//!
//! let state = Arc::new(AppState::new(store).await);
//! let config = ServerConfig::new("127.0.0.1:8788");
//! let mut bound = bind(&config).await?;
//!
//! println!("插件通道: {}", bound.socket_path().display());
//! bound.serve(state).await;
//! # Ok(())
//! # }
//! ```

pub mod assets;
pub mod error;
pub mod hub;
pub mod launch;
pub mod plugin;
pub mod plugin_socket;
pub mod poc;
pub mod server;
pub mod settings;
pub mod web;
pub mod web_ws;
pub mod wire;
