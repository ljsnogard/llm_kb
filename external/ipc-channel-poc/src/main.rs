//! servo/ipc-channel 0.23 可行性验证 PoC。
//!
//! 这个工程**不是** llm_kb 的一部分，只用来回答四个问题：
//!
//! 1. 跨进程能不能跑通（typed 通道 / bytes 通道 / 共享内存三套）；
//! 2. `String` / `Vec<u8>` 这类堆上内容能不能**不经任何类型转换**直接发送；
//! 3. 有没有**完全不需要 serde** 的通路；
//! 4. 阻塞的 `recv()` / `select()` 如何与异步运行时共存。
//!
//! 用法（由 `run-poc.sh` 驱动）：
//!
//! ```text
//! ipc-poc typed-server <name-file>   # 握手 + 回显 3 条（String/Vec）
//! ipc-poc typed-client <name-file>
//! ipc-poc bytes-server <name-file>   # 同样 3 条，但走 bytes_channel（零 serde）
//! ipc-poc bytes-client <name-file>
//! ipc-poc shm-server <name-file>     # 用 IpcSharedMemory 传 4 MiB（零用户 serde）
//! ipc-poc shm-client <name-file>
//! ipc-poc stream-demo                # IpcReceiver::to_stream() 在 tokio 多线程下消费
//! ipc-poc blocking-demo              # 对比「内联 recv()」与「to_stream()」
//! ```

use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use futures_lite::StreamExt;
use ipc_channel::ipc::{
    self, IpcBytesReceiver, IpcBytesSender, IpcOneShotServer, IpcReceiver, IpcSender,
    IpcSharedMemory,
};
use serde::{Deserialize, Serialize};

/// 请求：故意塞满「堆上内容」——`String` 与 `Vec<u8>`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Req {
    turn_id: String,
    question: String,
    /// 附件之类的二进制内容。
    blob: Vec<u8>,
    tags: Vec<String>,
}

/// 应答：同样带堆上内容。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Resp {
    turn_id: String,
    answer: String,
    echoed_blob_len: usize,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let arg = std::env::args().nth(2);

    match mode.as_str() {
        "typed-server" => typed_server(&arg.expect("需要 name 文件路径")),
        "typed-client" => typed_client(&arg.expect("需要 name 文件路径")),
        "bytes-server" => bytes_server(&arg.expect("需要 name 文件路径")),
        "bytes-client" => bytes_client(&arg.expect("需要 name 文件路径")),
        "shm-server" => shm_server(&arg.expect("需要 name 文件路径")),
        "shm-client" => shm_client(&arg.expect("需要 name 文件路径")),
        "stream-demo" => stream_demo(),
        "blocking-demo" => blocking_demo(),
        other => {
            eprintln!("unknown mode: {other:?}");
            Ok(())
        }
    }
}

/// 把 one-shot server 名字写到文件，供 driver 脚本转交给客户端进程。
fn publish_name(file: &str, name: &str) {
    fs::write(PathBuf::from(file), name).expect("写 name 文件失败");
}

fn read_name(file: &str) -> String {
    fs::read_to_string(PathBuf::from(file)).expect("读 name 文件失败").trim().to_string()
}

// ============================================================================
// 1. typed 通道：String / Vec 直发，不需要任何转换层
// ============================================================================

fn typed_server(name_file: &str) -> Result<(), Box<dyn std::error::Error>> {
    // 客户端会把「请求接收端」与「应答发送端」作为第一条消息送过来。
    let (server, name) = IpcOneShotServer::<(IpcReceiver<Req>, IpcSender<Resp>)>::new()?;
    publish_name(name_file, &name);

    let (_boot_rx, (req_rx, resp_tx)) = server.accept()?;

    for _ in 0..3 {
        let req = req_rx.recv()?;
        let resp = Resp {
            turn_id: req.turn_id.clone(),
            answer: format!("echo:{}", req.question),
            echoed_blob_len: req.blob.len(),
        };
        resp_tx.send(resp)?;
    }

    println!("SERVER-DONE typed requests=3");
    Ok(())
}

fn typed_client(name_file: &str) -> Result<(), Box<dyn std::error::Error>> {
    let name = read_name(name_file);

    let (req_tx, req_rx) = ipc::channel::<Req>()?;
    let (resp_tx, resp_rx) = ipc::channel::<Resp>()?;

    let boot = IpcSender::connect(name)?;
    boot.send((req_rx, resp_tx))?;

    let started = Instant::now();
    let mut answers = Vec::new();
    for index in 0..3u64 {
        let req = Req {
            turn_id: format!("t-{index}"),
            question: format!("第 {index} 个问题：你好，知识库"),
            blob: vec![index as u8; 4096],
            tags: vec!["中文标签".to_string(), "tag".to_string()],
        };
        req_tx.send(req)?;
        let resp = resp_rx.recv()?;
        answers.push(format!("{}/{}", resp.answer, resp.echoed_blob_len));
    }

    println!(
        "CLIENT-DONE typed round_trips={} elapsed={:?} first={:?}",
        answers.len(),
        started.elapsed(),
        answers.first()
    );
    Ok(())
}

// ============================================================================
// 2. bytes 通道：完全不走 serde 的原始字节通路
// ============================================================================

fn bytes_server(name_file: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (server, name) = IpcOneShotServer::<(IpcBytesReceiver, IpcBytesSender)>::new()?;
    publish_name(name_file, &name);

    let (_boot_rx, (req_rx, resp_tx)) = server.accept()?;

    let mut count = 0;
    let mut total = 0usize;
    while count < 3 {
        let bytes = req_rx.recv()?;
        total += bytes.len();
        // 完全由我们自己决定字节的含义，ipc-channel 不参与解释。
        let mut reply = Vec::with_capacity(bytes.len() + 8);
        reply.extend_from_slice(b"ACK:");
        reply.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        resp_tx.send(&reply)?;
        count += 1;
    }

    println!("SERVER-DONE bytes messages={count} total_bytes={total}");
    Ok(())
}

fn bytes_client(name_file: &str) -> Result<(), Box<dyn std::error::Error>> {
    let name = read_name(name_file);

    let (req_tx, req_rx) = ipc::bytes_channel()?;
    let (resp_tx, resp_rx) = ipc::bytes_channel()?;

    let boot = IpcSender::connect(name)?;
    boot.send((req_rx, resp_tx))?;

    let payloads: Vec<Vec<u8>> = vec![
        "你好，这是一条 UTF-8 原始字节".as_bytes().to_vec(),
        vec![0u8; 64 * 1024],
        vec![7u8; 4 * 1024 * 1024],
    ];

    let started = Instant::now();
    let mut acked = Vec::new();
    for payload in &payloads {
        req_tx.send(payload.as_slice())?;
        let reply = resp_rx.recv()?;
        acked.push((reply.len(), payload.len()));
    }

    println!(
        "CLIENT-DONE bytes sent={} elapsed={:?} acked={acked:?}",
        payloads.len(),
        started.elapsed()
    );
    Ok(())
}

// ============================================================================
// 3. IpcSharedMemory：把 4 MiB 放进共享内存再传「句柄」
// ============================================================================

fn shm_server(name_file: &str) -> Result<(), Box<dyn std::error::Error>> {
    // 注意：这条通道的消息类型只有 `IpcSharedMemory` 与 u64 之类，
    // **没有任何用户定义的 serde 类型**。
    let (server, name) =
        IpcOneShotServer::<(IpcSender<IpcSharedMemory>, IpcReceiver<IpcSharedMemory>)>::new()?;
    publish_name(name_file, &name);

    let (_boot_rx, (resp_tx, req_rx)) = server.accept()?;

    let shm = req_rx.recv()?;
    let len = shm.len();
    // Deref<Target = [u8]>：直接读共享内存，无中间拷贝。
    let checksum: u64 = shm.iter().fold(0u64, |acc, b| acc.wrapping_add(*b as u64));

    let reply_text = format!("len={len} checksum={checksum}");
    let reply = IpcSharedMemory::from_bytes(reply_text.as_bytes());
    resp_tx.send(reply)?;

    println!("SERVER-DONE shm received_bytes={len}");
    Ok(())
}

fn shm_client(name_file: &str) -> Result<(), Box<dyn std::error::Error>> {
    let name = read_name(name_file);

    let (resp_tx, resp_rx) = ipc::channel::<IpcSharedMemory>()?;
    let (req_tx, req_rx) = ipc::channel::<IpcSharedMemory>()?;

    let boot = IpcSender::connect(name)?;
    boot.send((resp_tx, req_rx))?;

    const SIZE: usize = 4 * 1024 * 1024;
    let started = Instant::now();
    let shm = IpcSharedMemory::from_byte(3, SIZE);
    let build = started.elapsed();

    let send_started = Instant::now();
    req_tx.send(shm)?;
    let send_elapsed = send_started.elapsed();

    let reply = resp_rx.recv()?;
    let text = String::from_utf8_lossy(&reply).to_string();

    println!(
        "CLIENT-DONE shm size={SIZE} build={build:?} send={send_elapsed:?} reply={text:?}"
    );
    Ok(())
}

// ============================================================================
// 4. 阻塞 IO 与异步运行时的共存
// ============================================================================

/// 正向：`IpcReceiver::to_stream()`（`async` feature）在 tokio 多线程下消费。
///
/// 阻塞的 `select()` 只发生在 ipc-channel 自己那个进程级 router 线程里，
/// 业务侧的异步任务完全不阻塞。
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn stream_demo() -> Result<(), Box<dyn std::error::Error>> {
    let (tx, rx) = ipc::channel::<u64>()?;
    let mut stream = rx.to_stream();

    let producer = std::thread::spawn(move || {
        for index in 0..500u64 {
            tx.send(index).expect("send 失败");
        }
    });

    // 同时跑一个 ticker，用来观察运行时有没有被卡住。
    let ticker = tokio::spawn(async move {
        let mut ticks = 0u64;
        for _ in 0..10 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            ticks += 1;
        }
        ticks
    });

    let started = Instant::now();
    let mut received = 0u64;
    while let Some(item) = stream.next().await {
        item?;
        received += 1;
    }
    let elapsed = started.elapsed();
    producer.join().expect("producer panic");
    let ticks = ticker.await?;

    println!("STREAM-DONE received={received} elapsed={elapsed:?} ticker_ticks={ticks}");
    Ok(())
}

/// 反向：在异步任务里**内联**调用阻塞的 `recv()`，观察运行时被饿死。
///
/// 用 `current_thread` 运行时把问题放大到确定性可见的程度。
#[tokio::main(flavor = "current_thread")]
async fn blocking_demo() -> Result<(), Box<dyn std::error::Error>> {
    const GAP: Duration = Duration::from_millis(300);

    // ── 阶段 1：内联 recv()（反模式） ──────────────────────────────
    let (tx1, rx1) = ipc::channel::<u64>()?;
    let ticks1 = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let counter1 = ticks1.clone();
    let ticker1 = tokio::spawn(async move {
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            counter1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    });
    std::thread::spawn(move || {
        std::thread::sleep(GAP);
        let _ = tx1.send(1);
    });

    let started = Instant::now();
    let value = rx1.recv()?; // ← 阻塞整个运行时线程
    let blocked = started.elapsed();
    let ticks_inline = ticks1.load(std::sync::atomic::Ordering::SeqCst);
    ticker1.abort();

    // ── 阶段 2：to_stream()（推荐做法） ────────────────────────────
    let (tx2, rx2) = ipc::channel::<u64>()?;
    let mut stream = rx2.to_stream();
    let ticks2 = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let counter2 = ticks2.clone();
    let ticker2 = tokio::spawn(async move {
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            counter2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    });
    std::thread::spawn(move || {
        std::thread::sleep(GAP);
        let _ = tx2.send(2);
    });

    let started = Instant::now();
    let value2 = stream.next().await.expect("stream 应当有数据")?;
    let awaited = started.elapsed();
    let ticks_stream = ticks2.load(std::sync::atomic::Ordering::SeqCst);
    ticker2.abort();

    println!(
        "BLOCKING-DEMO inline: value={value} elapsed={blocked:?} ticker_ticks={ticks_inline} \
         | stream: value={value2} elapsed={awaited:?} ticker_ticks={ticks_stream}"
    );
    Ok(())
}
