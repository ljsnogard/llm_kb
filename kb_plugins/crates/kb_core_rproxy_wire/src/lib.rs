//! # kb_core_rproxy_wire
//!
//! `kb_core_rproxy` 的 **TCP 帧格式**——服务端（网关自己）与客户端（任何远程
//! 访问者，含 `kb_admin_desktop`）共用同一份编解码。
//!
//! ```text
//! [u32 BE 长度][1 字节种类][postcard 载荷]
//!      长度 = 种类 + 载荷的字节数
//!      种类：0 = 请求（上行）；1 = 应答（下行）；2 = 事件（下行，预留）
//!      载荷：RequestEnvelope / ReplyEnvelope / Event 的 postcard 编码
//! ```
//!
//! # 为什么单独一个 crate
//!
//! 这段格式原来长在 `kb_core_rproxy` 的 `src/frame_.rs` 里，而那个 crate 只有
//! `[[bin]]`、没有 lib target。客户端要照着它实现时，只能**抄一份**——线格式
//! 两处各写一遍，改一处忘一处就是"跑起来才发现对端解不开"。
//!
//! 抽出来之后：
//!
//! | 谁 | 用哪些 |
//! | :--- | :--- |
//! | `kb_core_rproxy`（服务端） | [`decode_request`] + [`encode_reply`] / [`encode_event`] |
//! | 远程客户端 | [`encode_request`] + [`decode_frame`] |
//!
//! 这里**只有纯编解码**：不碰 socket、不依赖任何异步运行时（网关那边用 compio
//! 写 socket，客户端那边用 `std::net`，两边都只是把这里的字节搬来搬去）。
//!
//! # 示例
//!
//! ```
//! use abs_kb_svc_v1_desktop::{Request, RequestEnvelope};
//! use kb_core_rproxy_wire::{Frame, decode_frame, encode_request};
//!
//! let envelope = RequestEnvelope::new("q-1", Request::ListWorkspaces);
//! let mut buffer = encode_request(&envelope).expect("应当能编码");
//! assert_eq!(decode_frame(&mut buffer).expect("应当能解"), Some(Frame::Request(envelope)));
//! assert!(buffer.is_empty(), "用掉的字节应当被去掉");
//! ```

mod frame_;

pub use frame_::{
    Frame, KIND_EVENT, KIND_REPLY, KIND_REQUEST, MAX_FRAME_BYTES, decode_frame, decode_request,
    encode_event, encode_reply, encode_request,
};
