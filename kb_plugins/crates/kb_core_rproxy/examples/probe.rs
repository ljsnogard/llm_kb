//! 最小"远程客户端"：按 `kb_core_rproxy` 的 TCP 帧格式发两个请求，打印应答。
//!
//! 它的作用是**给这套网关一个可复现的验证手段**，也是"远程客户端长什么样"的
//! 参考实现：帧 = `[u32 BE 长度][1 字节种类][postcard 载荷]`，
//! 载荷就是 `abs_kb_svc::v1::desktop` 里的协议类型。
//!
//! 用法：
//!
//! ```bash
//! # 终端 1
//! cargo run -p kb_core_rproxy -- --listen 127.0.0.1:8788 --runtime-dir /tmp/kb-rp/run --storage-dir /tmp/kb-rp/data
//! # 终端 2
//! cargo run -p kb_core_rproxy --example probe -- 127.0.0.1:8788
//! ```

use std::io::{Read, Write};
use std::net::TcpStream;

use abs_kb_svc::v1::desktop::{
    ClientInfo, LocalId, PROTOCOL_VERSION, Reply, ReplyEnvelope, Request, RequestEnvelope,
};

/// 上行帧的种类。
const KIND_REQUEST: u8 = 0;

/// 下行帧的种类：应答。
const KIND_REPLY: u8 = 1;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8788".to_string());
    let mut stream = TcpStream::connect(&addr)?;
    println!("已连上 {addr}");

    // ── 应用层握手 ────────────────────────────────────────────────────
    let hello = RequestEnvelope::new(
        "q-hello",
        Request::Hello(ClientInfo {
            client_name: "kb_core_rproxy-probe".to_string(),
            client_version: "0.1.0".to_string(),
            protocol_version: PROTOCOL_VERSION,
        }),
    );
    match send_and_recv(&mut stream, &hello)? {
        Reply::Hello(info) => println!(
            "握手 OK：服务端版本 {}，协议 v{}",
            info.server_version, info.protocol_version
        ),
        other => println!("握手应答出乎意料: {other:?}"),
    }

    // ── 登记一个工作区，再列出来 ──────────────────────────────────────
    let add = RequestEnvelope::new(
        "q-add",
        Request::AddWorkspace(abs_kb_svc::v1::desktop::AddWorkspaceRequest {
            local_id: LocalId::new("l-probe"),
            name: "probe 建的工作区".to_string(),
            path: "/tmp/probe".to_string(),
        }),
    );
    match send_and_recv(&mut stream, &add)? {
        Reply::WorkspaceAdded { workspace, .. } => {
            println!(
                "已建立工作区: {} ({})",
                workspace.name, workspace.workspace_id
            )
        }
        other => println!("建工作区应答出乎意料: {other:?}"),
    }

    let list = RequestEnvelope::new("q-list", Request::ListWorkspaces);
    match send_and_recv(&mut stream, &list)? {
        Reply::WorkspaceList(list) => {
            println!("共 {} 个工作区", list.workspaces.len());
            for workspace in &list.workspaces {
                println!(
                    "  {}\t{}\t{}",
                    workspace.workspace_id, workspace.name, workspace.path
                );
            }
        }
        other => println!("列工作区应答出乎意料: {other:?}"),
    }

    Ok(())
}

/// 发一个请求信封、读回一个应答信封。
fn send_and_recv(
    stream: &mut TcpStream,
    envelope: &RequestEnvelope,
) -> Result<Reply, Box<dyn std::error::Error>> {
    write_frame(stream, KIND_REQUEST, &postcard::to_allocvec(envelope)?)?;
    let (kind, payload) = read_frame(stream)?;
    if kind != KIND_REPLY {
        return Err(format!("下行帧的种类不是应答: {kind}").into());
    }
    let reply: ReplyEnvelope = postcard::from_bytes(&payload)?;
    if reply.request_id != envelope.request_id {
        return Err(format!(
            "应答的 request_id 对不上: 期望 {}，实际 {}",
            envelope.request_id, reply.request_id
        )
        .into());
    }
    Ok(reply.reply)
}

/// 写一帧。
fn write_frame(
    stream: &mut TcpStream,
    kind: u8,
    payload: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let length = u32::try_from(payload.len() + 1)?;
    stream.write_all(&length.to_be_bytes())?;
    stream.write_all(&[kind])?;
    stream.write_all(payload)?;
    stream.flush()?;
    Ok(())
}

/// 读一帧。
fn read_frame(stream: &mut TcpStream) -> Result<(u8, Vec<u8>), Box<dyn std::error::Error>> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header) as usize;
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body)?;
    let kind = body[0];
    body.remove(0);
    Ok((kind, body))
}
