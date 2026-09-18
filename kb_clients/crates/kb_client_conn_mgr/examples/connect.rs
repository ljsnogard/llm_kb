//! 手工验证用的最小 CLI：按一条连接方式连上 `kb_core`，打印服务端身份与工作区 / 会话。
//!
//! 它同时是三种连接方式的**可运行样例**：
//!
//! ```bash
//! # 本机·启动（自己起一个 kb_core）
//! cargo run -p kb_client_conn_mgr --example connect -- \
//!     launch ./target/debug/kb-core /tmp/kb-cli/run /tmp/kb-cli/data
//!
//! # 本机·附着（连接已经在跑的 kb_core）
//! cargo run -p kb_client_conn_mgr --example connect -- attach /tmp/kb-cli/run
//!
//! # 远程（经 kb_core_rproxy）
//! cargo run -p kb_client_conn_mgr --example connect -- tcp 127.0.0.1:8788
//! ```
//!
//! 退出码：`0` 成功；`1` 连接或查询失败；`2` 参数错误。

use std::path::PathBuf;
use std::process::ExitCode;

use abs_cancel::TrMayCancel;
use futures_lite::future::block_on;
use kb_client_config::{Connection, default_storage_dir};
use kb_client_conn_mgr::{TimeoutToken, connect};

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let profile = match parse_(args) {
        Ok(profile) => profile,
        Err(message) => {
            eprintln!("参数错误: {message}");
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    match block_on(run_(profile)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("失败: {error}");
            ExitCode::FAILURE
        }
    }
}

/// 用法说明。
const USAGE: &str = "\
connect —— 按一条连接方式连上 kb_core 并列出工作区

用法:
    connect launch <kb-core 路径> <运行时目录> [存储目录]
        启动一个本机 kb_core，连上它。

    connect attach <运行时目录>
        连接已经在跑的本机 kb_core。

    connect tcp <主机:端口>
        经 kb_core_rproxy 连接远程 kb_core（无鉴权，仅受信网络）。
";

/// 解析命令行。
fn parse_(args: Vec<String>) -> Result<Connection, String> {
    let kind = args.first().map(String::as_str).unwrap_or("");
    match (kind, args.len()) {
        ("launch", 3) | ("launch", 4) => {
            let kb_core = PathBuf::from(&args[1]);
            let runtime_dir = PathBuf::from(&args[2]);
            let storage_dir = args
                .get(3)
                .map(PathBuf::from)
                .unwrap_or_else(|| default_storage_dir(&runtime_dir));
            Ok(Connection::local_launch(
                "launch",
                kb_core,
                runtime_dir,
                storage_dir,
            ))
        }
        ("attach", 2) => Ok(Connection::local_attach("attach", PathBuf::from(&args[1]))),
        ("tcp", 2) => Ok(Connection::tcp("tcp", args[1].clone())),
        _ => Err("参数个数或子命令不对".to_string()),
    }
}

/// ping 一条连接方式：连上、握手、列工作区与会话。
async fn run_(profile: Connection) -> Result<(), kb_client_conn_mgr::ClientError> {
    println!("连接方式: {} ({})", profile.name(), profile.kind().as_str());

    let client = connect(&profile)
        .may_cancel_with(TimeoutToken::after(
            profile.handshake_timeout().max(profile.connect_timeout()),
        ))
        .await?;

    println!(
        "已连上：服务端 {}，协议 v{}（本机={}，自起进程 pid={:?}）",
        client.server_info().server_version,
        client.server_info().protocol_version,
        client.is_local(),
        client.launched_pid()
    );

    let workspaces = client
        .list_workspaces()
        .may_cancel_with(TimeoutToken::after(profile.request_timeout()))
        .await?;
    println!("共 {} 个工作区", workspaces.workspaces.len());
    for workspace in &workspaces.workspaces {
        println!(
            "  {}\t{}\t{}",
            workspace.workspace_id, workspace.name, workspace.path
        );

        let sessions = client
            .list_sessions(workspace.workspace_id.clone())
            .may_cancel_with(TimeoutToken::after(profile.request_timeout()))
            .await?;
        println!("    共 {} 个会话", sessions.sessions.len());
        for session in &sessions.sessions {
            println!(
                "      {}\t{}\t{} 条\t更新于 {}",
                session.session_id, session.title, session.turn_count, session.updated_at_millis
            );
        }
    }

    // `client` 在这里被丢弃：`launch` 起的子进程会随之结束。
    Ok(())
}
