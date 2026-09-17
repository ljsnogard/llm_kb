//! 最小客户端示例：连上正在运行的 `kb_core`，列出已有工作区。
//!
//! 用法（两个终端，或把服务端放到后台）：
//!
//! ```bash
//! # 终端 1：起服务端
//! cargo run -p kb_core -- --runtime-dir /tmp/kb-demo/run --storage-dir /tmp/kb-demo/data
//!
//! # 终端 2：先塞一个工作区进去（临时子命令），再用本示例通过 IPC 读出来
//! cargo run -p kb_core -- --runtime-dir /tmp/kb-demo/run --storage-dir /tmp/kb-demo/data \
//!   workspace add --name 笔记 --path /tmp/notes
//! cargo run -p kb_svc_servo_ipc --example list_workspaces -- /tmp/kb-demo/run
//! ```
//!
//! 输出形如（每行是「标识 + 制表符 + 名字 + 制表符 + 路径」）：
//!
//! ```text
//! 已连上 /tmp/kb-demo/run
//! 共 1 个工作区
//! w-3f2b9c1d4e5a4b7c8d9e0f1a2b3c4d5e <TAB> 笔记 <TAB> /tmp/notes
//! ```

use abs_kb_svc::v1::desktop::TrWorkspaceService;
use kb_svc_servo_ipc::Client;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/kb-demo/run".to_string());

    let client = Client::connect(&runtime_dir)?;
    println!("已连上 {runtime_dir}");

    // `list_workspaces()` 返回的是 `IntoFuture`（可取消 future），`.await` 直接可用。
    let list = futures_lite::future::block_on(async { client.list_workspaces().await })?;

    println!("共 {} 个工作区", list.workspaces.len());
    for workspace in &list.workspaces {
        println!(
            "{}\t{}\t{}",
            workspace.workspace_id, workspace.name, workspace.path
        );
    }
    Ok(())
}
