//! 服务端的**接受循环**：公布端点、等客户端、把连接交出去。
//!
//! 这里刻意保持"一次只等一个客户端"的形状——`accept()` 是阻塞的，
//! 每接受一个就重建 one-shot 端点并把新端点名写回名字文件，客户端靠重试衔接。
//!
//! # 文件名是谁定的
//!
//! **`kb_core` 定**：`Listener::bind` 生成
//! `<runtime_dir>/kb-<YYYYMMDD>-<uuid-v4>.ipc`（见 [`super::rendezvous_`]），
//! 这个名字在整个进程生命周期里不变；变的只是文件**内容**（当前可连的端点名）。
//! 传输库（当前是 ipc-channel）只负责它自己那个 socket 放在哪，不参与命名。

use std::path::{Path, PathBuf};

use ipc_channel::ipc::IpcOneShotServer;

use super::connection_::{Bootstrap, Connection};
use super::error_::ServoIpcError;
use super::rendezvous_::{clear_name_, clear_stale_name_files_, new_name_file_in, publish_name_};

/// 服务端端点：把 `kb_core` 挂在某个运行时目录上，等待客户端连接。
///
/// # 与线程的关系
///
/// [`Listener::accept`] 是**阻塞**调用（内部是 `IpcOneShotServer::accept`），
/// 必须从阻塞线程调用——例如 compio 的 `spawn_blocking`，或一条专用线程。
/// `kb_core` 用的是前者。
pub struct Listener {
    /// 本次启动专属的端点名字文件。
    name_file_: PathBuf,
}

impl Listener {
    /// 在 `runtime_dir` 下准备端点。
    ///
    /// 做两件事：
    ///
    /// 1. 创建该目录（幂等），并**清掉上次运行残留的名字文件**——它们指向的端点
    ///    早就没了，留着只会让客户端对着死名字重试到超时；
    /// 2. 生成本次启动专属的名字文件：`kb-<日期>-<uuid>.ipc`。
    ///    此时还不写内容（没有在等连接）。
    ///
    /// 一个运行时目录只应当挂**一个** `kb_core`；两个实例会互相清名字。
    ///
    /// # Errors
    ///
    /// 目录建不出来时返回 [`ServoIpcError::RuntimeDir`]。
    pub fn bind(runtime_dir: impl Into<PathBuf>) -> Result<Self, ServoIpcError> {
        let runtime_dir = runtime_dir.into();
        std::fs::create_dir_all(&runtime_dir).map_err(|source| ServoIpcError::RuntimeDir {
            path: runtime_dir.clone(),
            source,
        })?;
        clear_stale_name_files_(&runtime_dir)?;

        let name_file = new_name_file_in(&runtime_dir);
        log::debug!("本次启动的端点名字文件: {}", name_file.display());
        Ok(Self {
            name_file_: name_file,
        })
    }

    /// 本次启动专属的端点名字文件路径。
    ///
    /// 名字里带日期与 UUID，所以每次启动都不同；换传输实现也保留这条约定。
    pub fn name_file(&self) -> &Path {
        &self.name_file_
    }

    /// 公布端点，并**阻塞**等待一个客户端连上来。
    ///
    /// 返回之后名字文件被清空（写入空串）：那个端点只能被消费一次，留着只会让
    /// 下一个客户端连到一个死端点。下一次调用会重建端点并重新写回内容。
    ///
    /// # Errors
    ///
    /// - [`ServoIpcError::CreateEndpoint`]：建不出 one-shot 端点；
    /// - [`ServoIpcError::NameFile`]：名字文件写不出去；
    /// - [`ServoIpcError::Transport`]：`accept` 本身失败。
    pub fn accept(&self) -> Result<Connection, ServoIpcError> {
        let (server, name) =
            IpcOneShotServer::<Bootstrap>::new().map_err(ServoIpcError::CreateEndpoint)?;
        publish_name_(&self.name_file_, &name)?;
        log::debug!("已公布端点 {}（等待客户端连接）", name);

        let outcome = server.accept();
        // 无论成败都先清空：这个名字已经被消费（或已经失效）。
        if let Err(error) = clear_name_(&self.name_file_) {
            log::warn!("清空端点名字文件失败: {error}");
        }

        let (_boot_rx, (request_rx, reply_tx, event_tx)) = outcome?;
        log::debug!("客户端已连接");
        Ok(Connection::new(request_rx, reply_tx, event_tx))
    }
}
