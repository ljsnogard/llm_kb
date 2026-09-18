//! 远程客户端：说 `kb_core_rproxy` 的 TCP 帧协议。
//!
//! # 形状
//!
//! 与 `kb_svc_servo_ipc::Client` 刻意同构——**阻塞全部收敛在自己的线程上，
//! future 里只有轮询**：
//!
//! ```text
//! send_envelope()  ──请求──► mpsc 通道 ──► 写线程 ──TCP──► 网关
//!        │                                                    │
//!        └── 登记 oneshot ◄── 读线程 ◄──── 应答帧 ◄────────────┘
//! ```
//!
//! 于是 tokio / compio / `futures_lite::block_on` 都能驱动它，也不需要为了一个
//! TCP 客户端引入某个异步运行时的 `net` 模块。
//!
//! # 一次一个请求
//!
//! 网关那侧是"收一帧 → 转发 → 写回 → 再收下一帧"，所以即使这里同时发多个请求，
//! 服务端也是排队处理的。客户端本身按 `request_id` 配对，应答乱序也能对上。
//! **流控**交给 TCP 自己：写线程写不进去就阻塞在 `write_all` 上。
//!
//! # 事件
//!
//! 下行帧的种类有三种（请求 / 应答 / 事件）。现在服务端还不推事件，但读线程
//! **按种类分派**，不会把事件帧错当应答解——等事件上线时这里只需接一个通道。

use std::collections::HashMap;
use std::future::Future;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use abs_cancel::TrCancellationToken;
use abs_kb_svc_v1_desktop::{ReplyEnvelope, Request, RequestEnvelope, RequestId, RpcError};
use futures_channel::oneshot;
use gen_mcf2::gen_may_cancel_future;
use kb_core_rproxy_wire::{Frame, decode_frame, encode_request};
use thiserror::Error;

/// 读 socket 时一次拿多少字节。
const READ_CHUNK_BYTES_: usize = 8 * 1024;

/// 远程 TCP 客户端的失败。
#[derive(Debug, Error)]
pub enum TcpError {
    /// 地址解析不出任何 `SocketAddr`。
    #[error("解析地址失败 {address:?}: {source}")]
    Resolve {
        /// 配置里写的地址。
        address: String,

        /// 底层错误。
        #[source]
        source: std::io::Error,
    },

    /// 地址解析成功但一个都没连上。
    #[error("连接 {address} 失败: {source}")]
    Connect {
        /// 配置里写的地址。
        address: String,

        /// 最后一个地址的失败原因。
        #[source]
        source: std::io::Error,
    },

    /// 地址字符串解析不出 `主机:端口`。
    #[error("地址 {address:?} 不是 `主机:端口` 的形式")]
    BadAddress {
        /// 配置里写的地址。
        address: String,
    },

    /// 底层读写失败。
    #[error("TCP I/O 失败: {0}")]
    Io(#[source] std::io::Error),

    /// 帧编解码失败。
    #[error("TCP 帧编解码失败: {0}")]
    Wire(#[source] std::io::Error),

    /// 对端关闭了连接。
    #[error("对端关闭了连接")]
    PeerClosed,

    /// 写线程已经不在了（连接坏了）。
    #[error("写线程已经退出，连接不可用")]
    WriterGone,

    /// 调用被取消。
    #[error("调用被取消")]
    Cancelled,

    /// 读 / 写线程起不来。
    #[error("起 TCP 工作线程失败: {0}")]
    WorkerSpawn(#[source] std::io::Error),
}

/// 远程 TCP 客户端。
pub struct TcpClient {
    /// 上行：请求交给写线程。
    request_tx_: Sender<RequestEnvelope>,

    /// 还没拿到应答的请求：`request_id` → 完成量。
    pending_: Arc<Mutex<HashMap<String, oneshot::Sender<ReplyEnvelope>>>>,

    /// 请求标识发号器（本连接内唯一）。
    next_request_: AtomicU64,
}

impl TcpClient {
    /// 连上网关。
    ///
    /// `connect_timeout` 是**每个地址**的连接时限（域名可能解析出多个地址，
    /// 逐个试）。这是**阻塞**调用，应当从合适的线程调用——本 crate 的连接管理器
    /// 会把它搬到专职线程上。
    ///
    /// # Errors
    ///
    /// - [`TcpError::BadAddress`] / [`TcpError::Resolve`]：地址不可用；
    /// - [`TcpError::Connect`]：所有地址都连不上；
    /// - [`TcpError::WorkerSpawn`]：读写线程起不来。
    pub fn connect(address: &str, connect_timeout: Duration) -> Result<Self, TcpError> {
        let addrs = address
            .to_socket_addrs()
            .map_err(|source| match source.kind() {
                std::io::ErrorKind::InvalidInput => TcpError::BadAddress {
                    address: address.to_string(),
                },
                _ => TcpError::Resolve {
                    address: address.to_string(),
                    source,
                },
            })?
            .collect::<Vec<_>>();

        if addrs.is_empty() {
            return Err(TcpError::BadAddress {
                address: address.to_string(),
            });
        }

        let mut last_error = None;
        let mut stream = None;
        for addr in &addrs {
            match TcpStream::connect_timeout(addr, connect_timeout) {
                Ok(connected) => {
                    stream = Some(connected);
                    break;
                }
                Err(source) => {
                    log::debug!("连接 {addr} 失败: {source}");
                    last_error = Some(source);
                }
            }
        }

        let stream = stream.ok_or_else(|| TcpError::Connect {
            address: address.to_string(),
            source: last_error.unwrap_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotConnected, "没有可用地址")
            }),
        })?;

        Self::from_stream(stream)
    }

    /// 用一条已经建立的流造客户端（测试里用假的网关）。
    ///
    /// # Errors
    ///
    /// 克隆句柄或起线程失败时返回 [`TcpError::Io`] / [`TcpError::WorkerSpawn`]。
    pub fn from_stream(stream: TcpStream) -> Result<Self, TcpError> {
        stream.set_nodelay(true).map_err(TcpError::Io)?;
        let read_half = stream.try_clone().map_err(TcpError::Io)?;

        let (request_tx, request_rx) = mpsc::channel::<RequestEnvelope>();
        let pending = Arc::new(Mutex::new(HashMap::new()));

        // 写线程：把编码好的帧写出去。写不进去只可能是连接坏了，直接收工。
        let mut write_half = stream;
        std::thread::Builder::new()
            .name("kb-core-tcp-writer".to_string())
            .spawn(move || {
                while let Ok(envelope) = request_rx.recv() {
                    let outcome = encode_request(&envelope)
                        .map_err(TcpError::Wire)
                        .and_then(|frame| write_half.write_all(&frame).map_err(TcpError::Io));
                    if let Err(error) = outcome {
                        log::warn!("向网关写请求失败，写线程退出: {error}");
                        break;
                    }
                }
            })
            .map_err(TcpError::WorkerSpawn)?;

        // 读线程：按种类分派下行帧，把应答交给对应的等待者。
        let reader_pending = Arc::clone(&pending);
        std::thread::Builder::new()
            .name("kb-core-tcp-reader".to_string())
            .spawn(move || read_loop_(read_half, reader_pending))
            .map_err(TcpError::WorkerSpawn)?;

        Ok(Self {
            request_tx_: request_tx,
            pending_: pending,
            next_request_: AtomicU64::new(0),
        })
    }

    /// 发一个请求并等它的应答（自动生成 `request_id`）。
    ///
    /// 返回的 future 与异步运行时无关，并且可以取消：
    ///
    /// ```text
    /// client.send_request(Request::ListWorkspaces).await
    /// client.send_request(Request::ListWorkspaces).may_cancel_with(token).await
    /// ```
    pub fn send_request<'f>(&'f self, request: Request) -> SendRequestAsync<'f, 'f> {
        SendRequestAsync::new(self, request)
    }

    /// 发一个**原始信封**并等它的应答（`request_id` 由调用方给）。
    ///
    /// 给"需要自己管配对"的调用方用；连接管理器走 [`TcpClient::send_request`]。
    pub fn send_envelope<'f>(&'f self, envelope: RequestEnvelope) -> SendEnvelopeAsync<'f, 'f> {
        SendEnvelopeAsync::new(self, envelope)
    }

    /// 发一个信封并等应答（内部实现）。
    async fn send_envelope_<C>(
        &self,
        envelope: RequestEnvelope,
        cancel: C,
    ) -> Result<ReplyEnvelope, RpcError<TcpError>>
    where
        C: TrCancellationToken,
    {
        // 已经取消就没必要打扰对端。
        if cancel.is_cancelled() {
            return Err(RpcError::Transport(TcpError::Cancelled));
        }

        let request_id = envelope.request_id.clone();
        let (completion, waiting) = oneshot::channel();
        lock_(&self.pending_).insert(request_id.to_string(), completion);

        if let Err(error) = self.request_tx_.send(envelope) {
            lock_(&self.pending_).remove(request_id.as_str());
            log::warn!("请求送不进写线程: {error}");
            return Err(RpcError::Transport(TcpError::WriterGone));
        }

        match await_reply_(waiting, cancel).await {
            Some(Ok(envelope)) => Ok(envelope),
            // 完成量被丢弃 = 读线程退出 = 对端没了。
            Some(Err(_dropped)) => Err(RpcError::Transport(TcpError::PeerClosed)),
            None => {
                lock_(&self.pending_).remove(request_id.as_str());
                Err(RpcError::Transport(TcpError::Cancelled))
            }
        }
    }
}

impl core::fmt::Debug for TcpClient {
    /// 只报"还有多少请求在等应答"，不打印端点（它们不可读）。
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("TcpClient")
            .field("pending_requests", &lock_(&self.pending_).len())
            .finish_non_exhaustive()
    }
}

/// [`TcpClient::send_request`] 的实现体：发一个请求（自动编号）。
#[gen_may_cancel_future(SendRequest, pub)]
async fn send_request_async<'c, C>(
    client: &'c TcpClient,
    request: Request,
    cancel: C,
) -> Result<ReplyEnvelope, RpcError<TcpError>>
where
    C: TrCancellationToken,
{
    let request_id = RequestId::new(format!(
        "q-{}",
        client.next_request_.fetch_add(1, Ordering::Relaxed)
    ));
    client
        .send_envelope_(RequestEnvelope::new(request_id, request), cancel)
        .await
}

/// [`TcpClient::send_envelope`] 的实现体：发一个原始信封。
#[gen_may_cancel_future(SendEnvelope, pub)]
async fn send_envelope_async<'c, C>(
    client: &'c TcpClient,
    envelope: RequestEnvelope,
    cancel: C,
) -> Result<ReplyEnvelope, RpcError<TcpError>>
where
    C: TrCancellationToken,
{
    client.send_envelope_(envelope, cancel).await
}

/// 读线程主体：按种类分派下行帧，直到对端断开或出错。
///
/// 退出时清空 `pending_`，所有还在等的调用者会因为完成量被丢弃而拿到
/// [`TcpError::PeerClosed`]。
fn read_loop_(
    mut stream: TcpStream,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<ReplyEnvelope>>>>,
) {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; READ_CHUNK_BYTES_];

    'reading: loop {
        let read = match stream.read(&mut chunk) {
            Ok(0) => {
                log::debug!("网关关闭了连接");
                break;
            }
            Ok(read) => read,
            Err(error) => {
                log::warn!("从网关读数据失败: {error}");
                break;
            }
        };
        buffer.extend_from_slice(&chunk[..read]);

        loop {
            match decode_frame(&mut buffer) {
                Ok(Some(Frame::Reply(envelope))) => {
                    let key = envelope.request_id.to_string();
                    match lock_(&pending).remove(&key) {
                        Some(completion) => {
                            // 等待者可能已经取消并走开了；送不到也无所谓。
                            let _ = completion.send(envelope);
                        }
                        None => log::debug!("收到无人认领的应答: {key}"),
                    }
                }
                Ok(Some(Frame::Event(event))) => {
                    // 事件还没有订阅者；先记一条日志，别把它当应答。
                    log::debug!("收到网关推送的事件（当前无人订阅）: {event:?}");
                }
                Ok(Some(Frame::Request(_))) => {
                    log::warn!("下行收到请求帧，忽略");
                }
                Ok(None) => break,
                Err(error) => {
                    log::warn!("下行帧解码失败，读线程退出: {error}");
                    break 'reading;
                }
            }
        }
    }

    lock_(&pending).clear();
}

/// 等应答，或在取消令牌触发时收手。
///
/// - `Some(Ok(..))`：收到应答；
/// - `Some(Err(..))`：完成量被丢弃（读线程退出 / 对端关闭）；
/// - `None`：取消令牌先触发。
async fn await_reply_<C>(
    receiver: oneshot::Receiver<ReplyEnvelope>,
    cancel: C,
) -> Option<Result<ReplyEnvelope, oneshot::Canceled>>
where
    C: TrCancellationToken,
{
    let cancellation = cancel.cancellation();
    let mut receiver = receiver;
    let mut cancellation = std::pin::pin!(cancellation);

    std::future::poll_fn(move |context| {
        if let std::task::Poll::Ready(outcome) = std::pin::Pin::new(&mut receiver).poll(context) {
            return std::task::Poll::Ready(Some(outcome));
        }
        if std::pin::Pin::new(&mut cancellation)
            .poll(context)
            .is_ready()
        {
            return std::task::Poll::Ready(None);
        }
        std::task::Poll::Pending
    })
    .await
}

/// 取锁，忽略"中毒"：临界区里不会 panic。
fn lock_<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
