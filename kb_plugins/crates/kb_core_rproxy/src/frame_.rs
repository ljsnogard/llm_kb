//! TCP 帧：`[u32 BE 长度][1 字节种类][postcard 载荷]`。
//!
//! 上行（远程客户端 → `kb_core`）只有请求；下行既可能是应答也可能是服务端主动
//! 推送的事件，所以**种类字段现在就留着**——等事件真正开始推送时不必改线格式。
//!
//! 载荷用 **postcard**：它正是 ipc-channel 内部用的编解码器，协议类型已经有
//! "用 postcard 真解码"的往返测试守着（见 `abs_kb_svc` 的 `envelope_.rs`）。
//!
//! 解码是**缓冲区驱动**的（[`decode_request`]）而不是直接读 socket：上行字节要
//! 先经过 [`crate::ring_`] 的有界环形缓冲，一帧可能分几次到达。

use std::io;

use abs_kb_svc::v1::desktop::{Event, ReplyEnvelope, RequestEnvelope};
use compio::io::{AsyncWrite, AsyncWriteExt};

/// 一帧载荷（不含长度头与种类字节）的上限。
///
/// 对端可以谎报长度，所以必须有上限，否则一个字节的头部就能让我们分配
/// 几个 G。8 MiB 对知识库的元数据请求足够宽裕。
pub const MAX_FRAME_BYTES: u32 = 8 * 1024 * 1024;

/// 上行：远程客户端发来的请求。
pub const KIND_REQUEST_: u8 = 0;

/// 下行：`kb_core` 的应答。
pub const KIND_REPLY_: u8 = 1;

/// 下行：`kb_core` 主动推送的事件。
///
/// 服务端目前还没有推送事件（生成相关域未落地），因此暂时没有写入方；
/// 保留常量是为了让线格式现在就是最终形状。
#[allow(dead_code)]
pub const KIND_EVENT_: u8 = 2;

/// 从缓冲区前端尝试解出一帧请求。
///
/// - `Ok(None)`：字节还不够一帧，调用方应当继续喂；
/// - `Ok(Some(envelope))`：解出一帧，并把用掉的字节从缓冲区里去掉；
/// - `Err(..)`：帧长度或种类不合法、或者 postcard 解不开。
///
/// # Errors
///
/// 见上。
pub fn decode_request(buffer: &mut Vec<u8>) -> io::Result<Option<RequestEnvelope>> {
    // 头至少要有 4 字节长度 + 1 字节种类。
    if buffer.len() < 5 {
        return Ok(None);
    }

    let length = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("帧长度不合法: {length}（上限 {MAX_FRAME_BYTES}）"),
        ));
    }

    let total = 4 + length as usize;
    if buffer.len() < total {
        return Ok(None);
    }

    let kind = buffer[4];
    if kind != KIND_REQUEST_ {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("上行帧的种类只能是 {KIND_REQUEST_}，收到 {kind}"),
        ));
    }

    let envelope = postcard::from_bytes(&buffer[5..total]).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("请求帧的 postcard 解码失败: {error}"),
        )
    })?;
    buffer.drain(..total);
    Ok(Some(envelope))
}

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
    let payload = postcard::to_allocvec(reply).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("应答帧的 postcard 编码失败: {error}"),
        )
    })?;
    write_frame_(writer, KIND_REPLY_, &payload).await
}

/// 写一帧事件（当前没有调用方，见 [`KIND_EVENT_`]）。
///
/// # Errors
///
/// 编码失败或底层 I/O 失败。
#[allow(dead_code)]
pub async fn write_event<W>(writer: &mut W, event: &Event) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let payload = postcard::to_allocvec(event).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("事件帧的 postcard 编码失败: {error}"),
        )
    })?;
    write_frame_(writer, KIND_EVENT_, &payload).await
}

/// 写一帧。
async fn write_frame_<W>(writer: &mut W, kind: u8, payload: &[u8]) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let length = u32::try_from(payload.len() + 1)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "帧太大，长度放不进 u32"))?;

    let mut frame = Vec::with_capacity(payload.len() + 5);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.push(kind);
    frame.extend_from_slice(payload);

    // `compio` 的 `BufResult` 是元组结构体（`BufResult(io::Result<T>, B)`），
    // 不是元组，所以取字段而不是解构。
    writer.write_all(frame).await.0
}

#[cfg(test)]
mod tests_ {
    use super::*;
    use abs_kb_svc::v1::desktop::{ClientInfo, PROTOCOL_VERSION, Request};

    /// 把一帧请求编成字节（测试用的"对端"）。
    fn encode_request_(envelope: &RequestEnvelope) -> Vec<u8> {
        let payload = postcard::to_allocvec(envelope).expect("应当能编码");
        let length = u32::try_from(payload.len() + 1).expect("长度应当放得下");
        let mut frame = Vec::new();
        frame.extend_from_slice(&length.to_be_bytes());
        frame.push(KIND_REQUEST_);
        frame.extend_from_slice(&payload);
        frame
    }

    /// 造一个握手请求。
    fn hello_() -> RequestEnvelope {
        RequestEnvelope::new(
            "q-1",
            Request::Hello(ClientInfo {
                client_name: "test".to_string(),
                client_version: "0.1.0".to_string(),
                protocol_version: PROTOCOL_VERSION,
            }),
        )
    }

    /// 测试**分片到达**时解码器会等而不是报错，攒够后能解出完整信封。
    ///
    /// - 手段：把一帧逐字节喂进缓冲区，每次调用 `decode_request`。
    /// - 判断：除最后一次外都返回 `Ok(None)`，最后一次解出的信封与原始相等，
    ///   且缓冲区被清空（用掉的字节被去掉）。
    #[test]
    fn decoder_waits_for_the_whole_frame_() {
        let envelope = hello_();
        let frame = encode_request_(&envelope);

        let mut buffer = Vec::new();
        for (index, byte) in frame.iter().enumerate() {
            buffer.push(*byte);
            let outcome = decode_request(&mut buffer).expect("不应当报错");
            if index + 1 < frame.len() {
                assert!(outcome.is_none(), "第 {} 字节就解出了帧", index + 1);
            } else {
                assert_eq!(outcome, Some(envelope.clone()));
            }
        }
        assert!(buffer.is_empty(), "用掉的字节应当被去掉");
    }

    /// 测试一帧之后紧跟另一帧时能连续解出。
    ///
    /// - 手段：把两帧拼在一个缓冲区里，连续调用两次 `decode_request`。
    /// - 判断：两次都解出，第一次是第一个信封，第二次是第二个，最后缓冲区为空。
    #[test]
    fn decoder_splits_consecutive_frames_() {
        let first = hello_();
        let second = RequestEnvelope::new("q-2", Request::ListWorkspaces);
        let mut buffer = encode_request_(&first);
        buffer.extend_from_slice(&encode_request_(&second));

        assert_eq!(decode_request(&mut buffer).expect("应当能解"), Some(first));
        assert_eq!(decode_request(&mut buffer).expect("应当能解"), Some(second));
        assert!(decode_request(&mut buffer).expect("应当能解").is_none());
        assert!(buffer.is_empty());
    }

    /// 测试长度不合法与种类不对都会明确报错。
    ///
    /// - 手段：分别构造"长度超过上限"与"种类字节是应答"的两帧。
    /// - 判断：都返回 `InvalidData`，而不是被当成合法帧放过去。
    #[test]
    fn decoder_rejects_bad_length_and_kind_() {
        let mut too_long = Vec::new();
        too_long.extend_from_slice(&(MAX_FRAME_BYTES + 1).to_be_bytes());
        too_long.push(KIND_REQUEST_);
        too_long.extend_from_slice(&[0u8; 8]);
        assert_eq!(
            decode_request(&mut too_long).expect_err("应当报错").kind(),
            io::ErrorKind::InvalidData
        );

        let envelope = hello_();
        let mut wrong_kind = encode_request_(&envelope);
        wrong_kind[4] = KIND_REPLY_;
        assert_eq!(
            decode_request(&mut wrong_kind)
                .expect_err("应当报错")
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
}
