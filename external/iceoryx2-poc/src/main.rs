//! iceoryx2 v0.9.3 可行性验证 PoC。
//!
//! 这个工程**不是** llm_kb 的一部分，只是用来在真实机器上回答三个问题：
//!
//! 1. 跨进程是否真的能跑通（同机两个进程，pub/sub 与 request/response）；
//! 2. 「复杂数据」的约束到底有多硬（哪些类型能过、哪些过不了，编译期报什么错）；
//! 3. 「大量数据」的代价（单次载荷上限、分片传输要写多少代码）。
//!
//! 用法（由 `run-poc.sh` 驱动）：
//!
//! ```text
//! ipc-poc pub        # 发布 100 条 TurnChunk
//! ipc-poc sub        # 订阅直到收到 100 条或超时
//! ipc-poc srv        # request-response 服务端
//! ipc-poc cli        # request-response 客户端
//! ipc-poc big-pub 4  # 单次 4 MiB 载荷发布
//! ipc-poc big-sub    # 接收单次大载荷
//! ```

use std::time::{Duration, Instant};

use iceoryx2::prelude::*;

/// pub/sub 服务名。iceoryx2 的 `ServiceName` 不允许斜杠以外的奇怪字符，
/// 且最终会落到文件系统上。
const SERVICE_PUBSUB: &str = "llm_kb_poc_pubsub";

/// request-response 服务名。
const SERVICE_REQRES: &str = "llm_kb_poc_reqres";

/// 单次大载荷服务名（切片载荷）。
const SERVICE_BIG: &str = "llm_kb_poc_big";

/// 变长载荷服务名（切片载荷）。
const SERVICE_VAR: &str = "llm_kb_poc_var";

/// 文本容量（固定）。
const TEXT_CAP: usize = 4096;

/// 单次大载荷容量：4 MiB。
const BIG_CAP: usize = 4 * 1024 * 1024;

/// 模拟一条流式增量帧：全部是定长字段，无堆指针，可直接映射到共享内存。
///
/// 注意：`turn_id` 是定长字节数组而不是 `String`；`text` 是定长数组而不是 `String`。
/// 这正是把现有 `abs_llm::v1` 语义类型搬上共享内存时必须付出的改造代价。
#[repr(C)]
#[derive(Debug, Clone, Copy, ZeroCopySend)]
struct TurnChunk {
    /// turn 标识的原始字节（UTF-8）。
    turn_id: [u8; 36],
    /// 逻辑分类（0=answer 1=reasoning 2=function_call …）。
    logic: u8,
    /// 序列号。
    seq: u64,
    /// 总条数。
    total: u64,
    /// `text` 中有效字节数。
    text_len: u32,
    /// 文本内容（定长）。
    text: [u8; TEXT_CAP],
}

impl TurnChunk {
    fn text_str(&self) -> &str {
        let len = (self.text_len as usize).min(TEXT_CAP);
        std::str::from_utf8(&self.text[..len]).unwrap_or("<invalid utf8>")
    }

    fn turn_id_str(&self) -> &str {
        let end = self.turn_id.iter().position(|&b| b == 0).unwrap_or(36);
        std::str::from_utf8(&self.turn_id[..end]).unwrap_or("<invalid>")
    }
}

/// request-response 的请求：模拟「kb_core → 插件」的下发指令。
#[repr(C)]
#[derive(Debug, Clone, Copy, ZeroCopySend)]
struct Command {
    /// 命令种类（0=ask 1=cancel 2=ping）。
    kind: u8,
    /// turn 标识。
    turn_id: [u8; 36],
    /// 问题正文（定长）。
    question: [u8; TEXT_CAP],
    /// 问题有效长度。
    question_len: u32,
}

/// request-response 的应答：模拟「插件 → kb_core」的应答。
#[repr(C)]
#[derive(Debug, Clone, Copy, ZeroCopySend)]
struct Reply {
    /// 0=ok 1=error。
    status: u8,
    /// 应答正文。
    body: [u8; TEXT_CAP],
    /// 有效长度。
    body_len: u32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args().nth(1).unwrap_or_default();

    match mode.as_str() {
        "pub" => run_pub(),
        "sub" => run_sub(),
        "srv" => run_srv(),
        "cli" => run_cli(),
        "big-pub" => run_big_pub(),
        "big-pub-default" => run_big_pub_with(false),
        "big-pub-static" => run_big_pub_with(true),
        "big-sub" => run_big_sub(),
        "var-pub" => run_var_pub(),
        "var-sub" => run_var_sub(),
        other => {
            eprintln!("unknown mode: {other:?}");
            Ok(())
        }
    }
}

// ── pub/sub ────────────────────────────────────────────────────────────────

fn run_pub() -> Result<(), Box<dyn std::error::Error>> {
    let node = NodeBuilder::new().create::<ipc::Service>()?;
    let service = node
        .service_builder(&SERVICE_PUBSUB.try_into()?)
        .publish_subscribe::<TurnChunk>()
        .max_publishers(4)
        .max_subscribers(8)
        .subscriber_max_buffer_size(16)
        .history_size(2)
        .open_or_create()?;

    let publisher = service.publisher_builder().create()?;

    let total = 100u64;
    for seq in 0..total {
        let text = format!("delta-{seq}-这是一个中文增量分片");
        let mut chunk = TurnChunk {
            turn_id: [0; 36],
            logic: (seq % 2) as u8,
            seq,
            total,
            text_len: text.len() as u32,
            text: [0; TEXT_CAP],
        };
        chunk.turn_id[..4].copy_from_slice(b"t-01");
        chunk.text[..text.len()].copy_from_slice(text.as_bytes());

        let sample = publisher.loan_uninit()?;
        let sample = sample.write_payload(chunk);
        sample.send()?;
        std::thread::sleep(Duration::from_millis(2));
    }

    println!("PUB-DONE total={total}");
    Ok(())
}

fn run_sub() -> Result<(), Box<dyn std::error::Error>> {
    let node = NodeBuilder::new().create::<ipc::Service>()?;
    let service = node
        .service_builder(&SERVICE_PUBSUB.try_into()?)
        .publish_subscribe::<TurnChunk>()
        .max_publishers(4)
        .max_subscribers(8)
        .subscriber_max_buffer_size(16)
        .history_size(2)
        .open_or_create()?;

    let subscriber = service.subscriber_builder().create()?;

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut received = 0u64;
    let mut first_text = String::new();
    let mut last_seq = 0u64;

    while Instant::now() < deadline {
        if let Some(sample) = subscriber.receive()? {
            if received == 0 {
                first_text = sample.text_str().to_string();
            }
            last_seq = sample.seq;
            received += 1;
            if received == 100 {
                break;
            }
        } else {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    println!(
        "SUB-DONE received={received} last_seq={last_seq} first_text={first_text:?} turn_id_ok={}",
        received > 0
    );
    Ok(())
}

// ── request / response ─────────────────────────────────────────────────────

fn run_srv() -> Result<(), Box<dyn std::error::Error>> {
    let node = NodeBuilder::new().create::<ipc::Service>()?;
    let service = node
        .service_builder(&SERVICE_REQRES.try_into()?)
        .request_response::<Command, Reply>()
        .max_servers(1)
        .max_clients(8)
        .max_active_requests_per_client(4)
        .max_response_buffer_size(8)
        .open_or_create()?;

    let server = service.server_builder().create()?;

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut handled = 0;
    while Instant::now() < deadline {
        if let Some(active_request) = server.receive()? {
            let question_len = (active_request.question_len as usize).min(TEXT_CAP);
            let question =
                String::from_utf8_lossy(&active_request.question[..question_len]).to_string();
            println!("SRV-GOT kind={} question={question:?}", active_request.kind);

            let body = format!("echo:{question}");
            let mut reply = Reply {
                status: 0,
                body: [0; TEXT_CAP],
                body_len: body.len() as u32,
            };
            reply.body[..body.len()].copy_from_slice(body.as_bytes());

            let response = active_request.loan_uninit()?;
            let response = response.write_payload(reply);
            response.send()?;
            handled += 1;
        } else {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    println!("SRV-DONE handled={handled}");
    Ok(())
}

fn run_cli() -> Result<(), Box<dyn std::error::Error>> {
    let node = NodeBuilder::new().create::<ipc::Service>()?;
    let service = node
        .service_builder(&SERVICE_REQRES.try_into()?)
        .request_response::<Command, Reply>()
        .max_servers(1)
        .max_clients(8)
        .max_active_requests_per_client(4)
        .max_response_buffer_size(8)
        .open_or_create()?;

    let client = service.client_builder().create()?;

    let question = "你好，请介绍一下知识库";
    let mut command = Command {
        kind: 0,
        turn_id: [0; 36],
        question: [0; TEXT_CAP],
        question_len: question.len() as u32,
    };
    command.turn_id[..4].copy_from_slice(b"t-01");
    command.question[..question.len()].copy_from_slice(question.as_bytes());

    let pending = client.send_copy(command)?;

    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Some(response) = pending.receive()? {
            let len = (response.body_len as usize).min(TEXT_CAP);
            let body = String::from_utf8_lossy(&response.body[..len]).to_string();
            println!("CLI-DONE status={} body={body:?}", response.status);
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    println!("CLI-TIMEOUT");
    Ok(())
}

// ── 大载荷：切片载荷（推荐路径，直接在共享内存里写，不经栈拷贝） ──────────

fn run_big_pub() -> Result<(), Box<dyn std::error::Error>> {
    run_big_pub_impl(true, false, "BIG-PUB-DONE")
}

/// `static_prealloc = true` 时用 `AllocationStrategy::Static` + 显式上限；
/// 两者都不给（`dynamic = false, static = false`）时只设上限、用默认策略。
fn run_big_pub_with(static_prealloc: bool) -> Result<(), Box<dyn std::error::Error>> {
    if static_prealloc {
        run_big_pub_impl(true, true, "BIG-PUB-STATIC-DONE")
    } else {
        // 完全不配置切片上限与分配策略：用来观察默认配置下的失败模式。
        run_big_pub_impl(false, false, "BIG-PUB-DEFAULT")
    }
}

fn run_big_pub_impl(
    configure: bool,
    static_prealloc: bool,
    label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let node = NodeBuilder::new().create::<ipc::Service>()?;
    let service = node
        .service_builder(&SERVICE_BIG.try_into()?)
        .publish_subscribe::<[u8]>()
        .max_publishers(1)
        .max_subscribers(1)
        .subscriber_max_buffer_size(1)
        .history_size(1)
        .open_or_create()?;

    let publisher_builder = service.publisher_builder();
    let publisher = if !configure {
        publisher_builder.create()?
    } else if static_prealloc {
        publisher_builder
            .initial_max_slice_len(BIG_CAP)
            .allocation_strategy(AllocationStrategy::Static)
            .create()?
    } else {
        publisher_builder
            // 切片载荷的「最大可租借长度」提示；不设置时默认上限很小，
            // 4 MiB 的 loan_slice_uninit 会以 LoanError::ExceedsMaxLoanSize 失败。
            .initial_max_slice_len(BIG_CAP)
            // 需要增长时按 2 的幂扩容（另一种是 BestFit / Static）。
            .allocation_strategy(AllocationStrategy::PowerOfTwo)
            .create()?
    };

    let started = Instant::now();
    let sample = match publisher.loan_slice_uninit(BIG_CAP) {
        Ok(sample) => sample,
        Err(err) => {
            println!("{label} loan_error={err:?}");
            return Ok(());
        }
    };
    // `write_from_fn` 直接在共享内存上按索引初始化，不产生 4 MiB 的栈/堆中转。
    let sample = sample.write_from_fn(|index| (index % 251) as u8);
    sample.send()?;
    println!(
        "{label} bytes={BIG_CAP} loan_write_send={:?}",
        started.elapsed()
    );

    // 留一点时间给订阅者把数据读走。
    std::thread::sleep(Duration::from_secs(3));
    Ok(())
}

fn run_big_sub() -> Result<(), Box<dyn std::error::Error>> {
    let node = NodeBuilder::new().create::<ipc::Service>()?;
    let service = node
        .service_builder(&SERVICE_BIG.try_into()?)
        .publish_subscribe::<[u8]>()
        .max_publishers(1)
        .max_subscribers(1)
        .subscriber_max_buffer_size(1)
        .history_size(1)
        .open_or_create()?;

    let subscriber = service.subscriber_builder().create()?;

    let started = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Some(sample) = subscriber.receive()? {
            let len = sample.len();
            let checksum: u64 = sample
                .iter()
                .enumerate()
                .map(|(index, byte)| ((*byte as u64) ^ ((index % 251) as u64)).wrapping_add(1))
                .fold(0u64, |acc, value| acc.wrapping_mul(31).wrapping_add(value));
            println!(
                "BIG-SUB-DONE len={len} recv={:?} sanity={}",
                started.elapsed(),
                checksum != 0
            );
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    println!("BIG-SUB-TIMEOUT");
    Ok(())
}

// ── 变长载荷：一次会话里发多条不同长度的字节串 ─────────────────────────────

fn run_var_pub() -> Result<(), Box<dyn std::error::Error>> {
    let node = NodeBuilder::new().create::<ipc::Service>()?;
    let service = node
        .service_builder(&SERVICE_VAR.try_into()?)
        .publish_subscribe::<[u8]>()
        .max_publishers(1)
        .max_subscribers(1)
        .subscriber_max_buffer_size(16)
        .history_size(1)
        .open_or_create()?;

    let publisher = service
        .publisher_builder()
        // 故意把初始切片上限设得极小（8 字节），让后面 64 KiB / 512 KiB 的载荷
        // 触发扩容路径：这正是 FAQ 里「Losing Dynamic Data」警告所对应的场景。
        .initial_max_slice_len(8)
        .allocation_strategy(AllocationStrategy::PowerOfTwo)
        .create()?;

    // 模拟「同一类语义事件、长度差别很大」：小增量分片 → 收尾时的大 JSON。
    let payloads = [
        r#"{"type":"delta","text":"你好"}"#.as_bytes().to_vec(),
        r#"{"type":"delta","text":"，这是一段稍长一点的增量文本"}"#
            .as_bytes()
            .to_vec(),
        format!(r#"{{"type":"final","body":"{}"}}"#, "长".repeat(64 * 1024)).into_bytes(),
        format!(r#"{{"type":"final","body":"{}"}}"#, "长".repeat(512 * 1024)).into_bytes(),
    ];

    for payload in &payloads {
        let sample = publisher.loan_slice_uninit(payload.len())?;
        let sample = sample.write_from_slice(payload.as_slice());
        sample.send()?;
        std::thread::sleep(Duration::from_millis(5));
    }

    println!("VAR-PUB-DONE count={}", payloads.len());
    Ok(())
}

fn run_var_sub() -> Result<(), Box<dyn std::error::Error>> {
    let node = NodeBuilder::new().create::<ipc::Service>()?;
    let service = node
        .service_builder(&SERVICE_VAR.try_into()?)
        .publish_subscribe::<[u8]>()
        .max_publishers(1)
        .max_subscribers(1)
        .subscriber_max_buffer_size(16)
        .history_size(1)
        .open_or_create()?;

    let subscriber = service.subscriber_builder().create()?;

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut lens = Vec::new();
    while Instant::now() < deadline && lens.len() < 4 {
        if let Some(sample) = subscriber.receive()? {
            lens.push(sample.len());
        } else {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    println!("VAR-SUB-DONE lens={lens:?}");
    Ok(())
}
