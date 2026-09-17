//! 常驻服务：把 `kb_core` 挂在运行时目录上，等客户端连上来。
//!
//! # 形状
//!
//! ```text
//! Store::open(存储目录)          ← 本地文件存储（将来换成 Turso）
//! KbService::new(store)          ← 按域 RPC trait 的服务端实现
//! Listener::bind(运行时目录)     ← 公布端点
//! loop {
//!     spawn_blocking(accept)     ← accept 是阻塞调用，不占执行器线程
//!     connection.serve(service)  ← 异步：请求流由 ipc-channel 的 router 线程供给
//! }
//! ```
//!
//! 第一版**一次只服务一个客户端**：上一个断开之后才回到 `accept`。
//! 这够桌面客户端用，也让"端点重建 + 名字重发"这条引导机制保持简单；
//! 并发处理多个连接留给后续（见 `README.md` 的"下一轮"）。
//!
//! 终止方式仍然是 `Ctrl-C`：本进程没有安装信号处理器，走操作系统的默认处置。

use std::sync::Arc;

use abs_kb_svc::v1::desktop::PROTOCOL_VERSION;
use kb_svc_servo_ipc::Listener;
use log::{info, warn};

use crate::args_::Paths;
use crate::error_::CoreError;
use crate::ipc_::KbService;
use crate::store_::Store;

/// 常驻运行，直到进程收到 `Ctrl-C`。
///
/// # Errors
///
/// 存储打不开、端点建不出来时返回 [`CoreError`]。
/// 单个工作区文件坏掉**不会**阻止启动，只在日志里给一条警告；
/// 单个客户端连接异常结束也不会让服务退出，只记一条警告后继续等下一位。
pub async fn run(paths: &Paths) -> Result<(), CoreError> {
    let store = Store::open(&paths.storage_dir).await?;
    let service = KbService::new(store);
    let listener = Arc::new(Listener::bind(&paths.runtime_dir)?);

    info!(
        "kb_core v{}（协议 v{PROTOCOL_VERSION}）",
        env!("CARGO_PKG_VERSION")
    );
    info!("运行时目录: {}", paths.runtime_dir.display());
    info!("存储目录: {}", service.store().root().display());
    info!("IPC 端点文件: {}", listener.name_file().display());
    match service.store().list_workspaces().await {
        Ok(list) => info!("已登记工作区: {} 个", list.workspaces.len()),
        Err(error) => warn!("读取工作区失败（不影响启动）: {error}"),
    }
    info!("等待客户端连接（Ctrl-C 退出）");

    loop {
        let accepting = Arc::clone(&listener);
        // `accept()` 是阻塞调用，必须离开执行器线程——这正是它会阻塞在这里的原因。
        let connection = compio::runtime::spawn_blocking(move || accepting.accept())
            .await
            .map_err(|error| CoreError::BlockingTask(error.to_string()))??;

        info!("客户端已连接");
        match connection.serve(&service).await {
            Ok(()) => info!("客户端已断开，继续等待下一位"),
            Err(error) => warn!("本次连接异常结束: {error}"),
        }
    }
}
