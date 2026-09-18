//! compio 侧的帧 IO。
//!
//! 帧**格式**本身在 `kb_core_rproxy_wire`（服务端与客户端共用同一份编解码）；
//! 这里只负责把编出来的字节写进 compio 的 socket 写半部，以及把上行解码函数
//! 转出来给 `main.rs` 用。
//!
//! 解码是**缓冲区驱动**的（[`decode_request`]）而不是直接读 socket：上行字节要
//! 先经过 [`crate::ring_`] 的有界环形缓冲，一帧可能分几次到达。

use std::io;

use abs_kb_svc::v1::desktop::{Event, ReplyEnvelope};
use compio::io::{AsyncWrite, AsyncWriteExt};

pub use kb_core_rproxy_wire::decode_request;

/// 写一帧应答。
///
/// 泛型是为了同时接受整条 `TcpStream` 与 `split()` 出来的写半部。
///
/// # Errors
///
/// 编码失败或底层 I/O 失败。
pub async fn write_reply<W>(writer: &mut W, reply: &ReplyEnvelope) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let frame = kb_core_rproxy_wire::encode_reply(reply)?;
    // `compio` 的 `BufResult` 是元组结构体（`BufResult(io::Result<T>, B)`），
    // 不是元组，所以取字段而不是解构。
    writer.write_all(frame).await.0
}

/// 写一帧事件（当前没有调用方，见 `kb_core_rproxy_wire::KIND_EVENT`）。
///
/// # Errors
///
/// 编码失败或底层 I/O 失败。
#[allow(dead_code)]
pub async fn write_event<W>(writer: &mut W, event: &Event) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let frame = kb_core_rproxy_wire::encode_event(event)?;
    writer.write_all(frame).await.0
}
