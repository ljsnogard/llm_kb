//! 服务端的**接受循环**：公布端点、等客户端、把连接交出去。
//!
//! 这里刻意保持"一次只等一个客户端"的形状——`accept()` 是阻塞的，
//! 每接受一个就重建 one-shot server 并重发名字，客户端靠重试衔接。

use std::path::PathBuf;

use ipc_channel::ipc::IpcOneShotServer;

use super::connection_::{Bootstrap, Connection};
use super::error_::ServoIpcError;
use super::rendezvous_::{name_file_in, publish_name_, withdraw_name_};

/// 服务端端点：把 `kb_core` 挂在某个运行时目录上，等待客户端连接。
///
/// # 与线程的关系
///
/// [`Listener::accept`] 是**阻塞**调用（内部是 `IpcOneShotServer::accept`），
/// 必须从阻塞线程调用——例如 compio 的 `spawn_blocking`，或一条专用线程。
/// `kb_core` 用的是前者。
pub struct Listener {
    /// 端点名字文件的位置。
    name_file_: PathBuf,
}

impl Listener {
    /// 在 `runtime_dir` 下准备端点。
    ///
    /// 会创建该目录（幂等），并**清掉可能残留的名字文件**：上一次进程被强杀时
    /// 名字文件会留在那里，而它指向的端点早就没了——不清理的话，客户端会一直
    /// 对着一个死名字重试到超时。既然本进程就是服务端，启动时那句名字一定无效。
    ///
    /// 一个运行时目录只应当挂**一个** `kb_core`；两个实例会互相踩名字。
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
        let name_file = name_file_in(&runtime_dir);
        withdraw_name_(&name_file)?;
        Ok(Self {
            name_file_: name_file,
        })
    }

    /// 端点名字文件的位置。
    pub fn name_file(&self) -> &std::path::Path {
        &self.name_file_
    }

    /// 公布端点，并**阻塞**等待一个客户端连上来。
    ///
    /// 返回之后名字文件已经被撤下：这个名字只能被消费一次，留着只会让下一个
    /// 客户端连到一个死端点。下一次调用会重建并重新公布。
    ///
    /// # Errors
    ///
    /// - [`ServoIpcError::CreateEndpoint`]：建不出 one-shot server；
    /// - [`ServoIpcError::NameFile`]：名字文件写不出去；
    /// - [`ServoIpcError::Transport`]：`accept` 本身失败。
    pub fn accept(&self) -> Result<Connection, ServoIpcError> {
        let (server, name) =
            IpcOneShotServer::<Bootstrap>::new().map_err(ServoIpcError::CreateEndpoint)?;
        publish_name_(&self.name_file_, &name)?;
        log::debug!("已公布端点 {}（等待客户端连接）", name);

        let outcome = server.accept();
        // 无论成败都先撤下名字：这个名字已经被消费（或已经失效）。
        if let Err(error) = withdraw_name_(&self.name_file_) {
            log::warn!("撤销端点名字失败: {error}");
        }

        let (_boot_rx, (request_rx, reply_tx, event_tx)) = outcome?;
        log::debug!("客户端已连接");
        Ok(Connection::new(request_rx, reply_tx, event_tx))
    }
}
