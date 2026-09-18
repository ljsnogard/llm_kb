//! TCP 帧：`[u32 BE 长度][1 字节种类][postcard 载荷]`。
//!
//! 长度 = 种类字节 + 载荷的字节数；单帧载荷上限 [`MAX_FRAME_BYTES`]。
//!
//! 三个方向各有一种载荷（[`KIND_REQUEST`] / [`KIND_REPLY`] / [`KIND_EVENT`]），
//! 所以**解码不能假定"读到的下一帧就是我的应答"**：上行是请求，下行既可能是
//! 应答、也可能是服务端主动推送的事件。
//!
//! 载荷用 **postcard**：它正是 ipc-channel 内部用的编解码器，协议类型已经有
//! "用 postcard 真解码"的往返测试守着（见 `abs_kb_svc_v1_desktop` 的
//! `envelope_.rs`）。
//!
//! # 为什么是"缓冲区驱动"的解码
//!
//! 一帧可能分几次到达（TCP 是字节流），所以 [`decode_frame`] / [`decode_request`]
//! 不直接读 socket，而是先看缓冲区里够不够一帧：不够就返回 `Ok(None)` 让调用方
//! 继续喂字节。

use std::io;

use abs_kb_svc_v1_desktop::{Event, ReplyEnvelope, RequestEnvelope};

/// 一帧载荷（不含长度头与种类字节）的上限。
///
/// 对端可以谎报长度，所以必须有上限，否则一个字节的头部就能让我们分配
/// 几个 G。8 MiB 对知识库的元数据请求足够宽裕。
pub const MAX_FRAME_BYTES: u32 = 8 * 1024 * 1024;

/// 上行：远程客户端发来的请求。
pub const KIND_REQUEST: u8 = 0;

/// 下行：`kb_core` 的应答。
pub const KIND_REPLY: u8 = 1;

/// 下行：`kb_core` 主动推送的事件。
///
/// 服务端目前还没有推送事件（生成相关域未落地），因此暂时没有写入方；
/// 保留常量是为了让线格式现在就是最终形状。
pub const KIND_EVENT: u8 = 2;

/// 解出来的一帧。
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// 上行请求。
    Request(RequestEnvelope),

    /// 下行应答。
    Reply(ReplyEnvelope),

    /// 下行事件。
    Event(Event),
}

/// 从缓冲区前端尝试解出一帧（不限定方向）。
///
/// - `Ok(None)`：字节还不够一帧，调用方应当继续喂；
/// - `Ok(Some(frame))`：解出一帧，并把用掉的字节从缓冲区里去掉；
/// - `Err(..)`：帧长度或种类不合法、或者 postcard 解不开。
///
/// # Errors
///
/// 见上。
pub fn decode_frame(buffer: &mut Vec<u8>) -> io::Result<Option<Frame>> {
    let Some((kind, payload)) = split_frame_(buffer)? else {
        return Ok(None);
    };

    let frame = match kind {
        KIND_REQUEST => Frame::Request(decode_payload_(payload, "请求")?),
        KIND_REPLY => Frame::Reply(decode_payload_(payload, "应答")?),
        KIND_EVENT => Frame::Event(decode_payload_(payload, "事件")?),
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "帧的种类不认识: {other}（已知 {} / {} / {}）",
                    KIND_REQUEST, KIND_REPLY, KIND_EVENT
                ),
            ));
        }
    };

    Ok(Some(frame))
}

/// 从缓冲区前端尝试解出**一帧请求**（上行专用）。
///
/// 与 [`decode_frame`] 的区别是它把"种类不是请求"直接当成错误——网关这一侧
/// 收到非请求帧说明对端坏了，不该被静默忽略。
///
/// # Errors
///
/// 帧长度或种类不合法、或者 postcard 解不开。
pub fn decode_request(buffer: &mut Vec<u8>) -> io::Result<Option<RequestEnvelope>> {
    let Some((kind, payload)) = split_frame_(buffer)? else {
        return Ok(None);
    };

    if kind != KIND_REQUEST {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("上行帧的种类只能是 {KIND_REQUEST}，收到 {kind}"),
        ));
    }

    decode_payload_(payload, "请求").map(Some)
}

/// 把一帧请求编成字节（含长度头与种类字节）。
///
/// # Errors
///
/// postcard 编码失败，或载荷超过 [`MAX_FRAME_BYTES`]。
pub fn encode_request(envelope: &RequestEnvelope) -> io::Result<Vec<u8>> {
    encode_frame_(KIND_REQUEST, envelope, "请求")
}

/// 把一帧应答编成字节。
///
/// # Errors
///
/// 同 [`encode_request`]。
pub fn encode_reply(envelope: &ReplyEnvelope) -> io::Result<Vec<u8>> {
    encode_frame_(KIND_REPLY, envelope, "应答")
}

/// 把一帧事件编成字节。
///
/// # Errors
///
/// 同 [`encode_request`]。
pub fn encode_event(event: &Event) -> io::Result<Vec<u8>> {
    encode_frame_(KIND_EVENT, event, "事件")
}

/// 从缓冲区前端切出一帧，并把它从缓冲区里去掉。
///
/// `Ok(None)` 表示"还不够一帧"。切出来之后**必须**由调用方使用，否则那些字节
/// 就丢了——这也是为什么这个私有函数只在两个公开解码函数里调用。
fn split_frame_(buffer: &mut Vec<u8>) -> io::Result<Option<(u8, Vec<u8>)>> {
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
    let payload = buffer[5..total].to_vec();
    buffer.drain(..total);

    Ok(Some((kind, payload)))
}

/// 把 `kind` + postcard 载荷拼成一帧。
fn encode_frame_<T>(kind: u8, payload: &T, what: &'static str) -> io::Result<Vec<u8>>
where
    T: serde::Serialize,
{
    let bytes = postcard::to_allocvec(payload).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{what}帧的 postcard 编码失败: {error}"),
        )
    })?;

    let length = u32::try_from(bytes.len() + 1)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "帧太大，长度放不进 u32"))?;
    if length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("帧太大: {length} 字节（上限 {MAX_FRAME_BYTES}）"),
        ));
    }

    let mut frame = Vec::with_capacity(bytes.len() + 5);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.push(kind);
    frame.extend_from_slice(&bytes);
    Ok(frame)
}

/// 解一帧的载荷。
fn decode_payload_<T>(payload: Vec<u8>, what: &'static str) -> io::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    postcard::from_bytes(&payload).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{what}帧的 postcard 解码失败: {error}"),
        )
    })
}

#[cfg(test)]
mod tests_ {
    use super::*;
    use abs_kb_svc_v1_desktop::{
        ClientInfo, PROTOCOL_VERSION, Reply, Request, RequestId, ServerInfo,
    };

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

    /// 测试**分片到达**时解码器会等而不是报错，攒够后能解出完整帧。
    ///
    /// - 手段：把一帧请求逐字节喂进缓冲区，每次调用 `decode_request`。
    /// - 判断：除最后一次外都返回 `Ok(None)`，最后一次解出的信封与原始相等，
    ///   且缓冲区被清空（用掉的字节被去掉）。
    #[test]
    fn decoder_waits_for_the_whole_frame_() {
        let envelope = hello_();
        let frame = encode_request(&envelope).expect("应当能编码");

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
        let mut buffer = encode_request(&first).expect("应当能编码");
        buffer.extend_from_slice(&encode_request(&second).expect("应当能编码"));

        assert_eq!(decode_request(&mut buffer).expect("应当能解"), Some(first));
        assert_eq!(decode_request(&mut buffer).expect("应当能解"), Some(second));
        assert!(decode_request(&mut buffer).expect("应当能解").is_none());
        assert!(buffer.is_empty());
    }

    /// 测试长度不合法与种类不对都会明确报错。
    ///
    /// - 手段：分别构造"长度超过上限"、"长度是 0"、"上行帧种类是应答"三种输入。
    /// - 判断：都返回 `InvalidData`，而不是被当成合法帧放过去。
    #[test]
    fn decoder_rejects_bad_length_and_kind_() {
        let mut too_long = Vec::new();
        too_long.extend_from_slice(&(MAX_FRAME_BYTES + 1).to_be_bytes());
        too_long.push(KIND_REQUEST);
        too_long.extend_from_slice(&[0u8; 8]);
        assert_eq!(
            decode_request(&mut too_long).expect_err("应当报错").kind(),
            io::ErrorKind::InvalidData
        );

        let mut zero_length = Vec::new();
        zero_length.extend_from_slice(&0u32.to_be_bytes());
        zero_length.push(KIND_REQUEST);
        assert_eq!(
            decode_request(&mut zero_length)
                .expect_err("应当报错")
                .kind(),
            io::ErrorKind::InvalidData
        );

        let envelope = hello_();
        let mut wrong_kind = encode_request(&envelope).expect("应当能编码");
        wrong_kind[4] = KIND_REPLY;
        assert_eq!(
            decode_request(&mut wrong_kind)
                .expect_err("应当报错")
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    /// 测试通用解码按种类分派：请求 / 应答 / 事件各归各位。
    ///
    /// - 手段：分别编码一帧请求、一帧应答、一帧事件，拼在一起后用 `decode_frame`
    ///   连续解三次。
    /// - 判断：依次得到 [`Frame::Request`] / [`Frame::Reply`] / [`Frame::Event`]，
    ///   且载荷与原始值相等。这条钉住"客户端不能假定下一帧一定是应答"。
    #[test]
    fn decode_frame_dispatches_by_kind_() {
        let request = hello_();
        let reply = ReplyEnvelope::new(
            "q-1",
            Reply::Hello(ServerInfo {
                server_version: "0.1.0".to_string(),
                protocol_version: PROTOCOL_VERSION,
            }),
        );
        let event = Event::StateChanged(Default::default());

        let mut buffer = encode_request(&request).expect("应当能编码");
        buffer.extend_from_slice(&encode_reply(&reply).expect("应当能编码"));
        buffer.extend_from_slice(&encode_event(&event).expect("应当能编码"));

        assert_eq!(
            decode_frame(&mut buffer).expect("应当能解"),
            Some(Frame::Request(request))
        );
        assert_eq!(
            decode_frame(&mut buffer).expect("应当能解"),
            Some(Frame::Reply(reply))
        );
        assert_eq!(
            decode_frame(&mut buffer).expect("应当能解"),
            Some(Frame::Event(event))
        );
        assert!(decode_frame(&mut buffer).expect("应当能解").is_none());
        assert!(buffer.is_empty());
    }

    /// 测试未知的种类值会被明确拒绝。
    ///
    /// - 手段：把一帧请求的种类字节改成 99。
    /// - 判断：`decode_frame` 报 `InvalidData`，错误信息里带上那个值——
    ///   将来线格式扩展时，"老程序收到新种类"能一眼看出原因。
    #[test]
    fn decode_frame_rejects_unknown_kind_() {
        let mut frame = encode_request(&hello_()).expect("应当能编码");
        frame[4] = 99;

        let error = decode_frame(&mut frame).expect_err("应当报错");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("99"), "实际: {error}");
    }

    /// 测试应答的 `request_id` 会原样往返。
    ///
    /// - 手段：编码一个载荷是结构体的应答（`Reply::WorkspaceList`），解回来。
    /// - 判断：解出的 `request_id` 与载荷都与原始相等——客户端靠它把应答配回请求。
    #[test]
    fn reply_keeps_request_id_() {
        let reply = ReplyEnvelope::new(
            RequestId::new("q-42"),
            Reply::WorkspaceList(Default::default()),
        );
        let mut buffer = encode_reply(&reply).expect("应当能编码");
        let decoded = decode_frame(&mut buffer).expect("应当能解").expect("有帧");

        assert_eq!(decoded, Frame::Reply(reply));
    }
}
