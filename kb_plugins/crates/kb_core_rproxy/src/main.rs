//! # kb_core_rproxy
//!
//! **`kb_core` 的局域网应用层网关**：启动一个 `kb_core` 实例，自己监听一个 TCP
//! 地址，把远程访问者当作"格式与 `kb_svc_servo_ipc` 客户端相同的客户端"，
//! 把它的请求经 IPC 转给 `kb_core`，把应答原样送回。
//!
//! ```text
//! 远程客户端 ──TCP──► kb_core_rproxy ──IPC──► kb_core（本机）
//!                    （本进程）              （子进程）
//! ```
//!
//! ## ⚠️ 没有鉴权、没有 TLS
//!
//! 这个网关存在的目的是**局域网跨机测试**，所以它默认监听 `0.0.0.0:8788`
//! 而不是只绑 `127.0.0.1`。代价是**任何能访问到这个端口的人都能读写知识库**。
//!
//! 在受信网络之外使用之前，必须先做鉴权；这条不是"以后顺手加"的东西。
//!
//! ## 两层握手，各管一段
//!
//! | 层面 | 谁管 | 在本 crate 里的位置 |
//! | :--- | :--- | :--- |
//! | **系统层**（找得到、连得上） | 传输实现 | [`kb_core_starter`]：用 `--handshake-prompt=stdio` 启动 `kb_core`，读它公布的一行通知拿到 IPC 端点文件名 |
//! | **应用层**（谈得成） | `abs_kb_svc` 的协议（`Request::Hello`） | `main.rs`：**等真的有远程客户端连上来**才发起；启动时不打扰 `kb_core` |
//!
//! 分工的完整说明见 `abs_kb_svc::v1::desktop::handshake_` 的模块文档。
//!
//! ## TCP 帧
//!
//! `[u32 BE 长度][1 字节种类][postcard 载荷]`，载荷是 `RequestEnvelope` /
//! `ReplyEnvelope` / `Event`——**协议类型一个都不用改**，换的是搬运方式。
//! 细节见 [`frame_`]。
//!
//! ## 一次只服务一个客户端
//!
//! 上一版是"一次一个"：接受一个远程客户端、把它伺候到断开，再接受下一个。
//! 并发/多客户端要等这一版跑通、并且 `kb_core` 那侧能并发服务连接之后再说。

mod frame_;
mod ring_;

use std::path::PathBuf;
use std::process::ExitCode;

use abs_kb_svc::v1::desktop::{
    ClientInfo, PROTOCOL_VERSION, Reply, ReplyEnvelope, RpcError, TrHandshake,
};
use compio::io::AsyncRead;
use kb_core_starter::{LaunchSpec, start};
use kb_svc_servo_ipc::Client;

/// 缺省监听地址。
///
/// **刻意不是 `127.0.0.1`**：本网关就是为了局域网跨机测试。
/// 想只给本机用时显式传 `--listen 127.0.0.1:8788`。
const DEFAULT_LISTEN: &str = "0.0.0.0:8788";

/// 命令行用法。
const USAGE: &str = "\
kb-core-rproxy —— kb_core 的局域网应用层网关（无鉴权，勿用于不受信网络）

用法:
    kb-core-rproxy [选项]

选项:
    --listen <地址:端口>   监听地址，缺省 0.0.0.0:8788
    --kb-core <路径>       kb_core 可执行文件，缺省与本程序同目录下的 kb-core
    --runtime-dir <目录>   传给 kb_core 的运行时目录（缺省 $XDG_RUNTIME_DIR/llm_kb）
    --storage-dir <目录>   传给 kb_core 的数据目录（缺省 <运行时目录>/storage）
    -h, --help             打印本说明
";

/// 解析后的命令行。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Options {
    /// 监听地址。
    listen_: String,

    /// `kb_core` 可执行文件。
    kb_core_: PathBuf,

    /// 运行时目录。
    runtime_dir_: PathBuf,

    /// 数据目录。
    storage_dir_: PathBuf,
}

/// 参数错误。
#[derive(Debug)]
struct UsageError_(String);

impl core::fmt::Display for UsageError_ {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl core::error::Error for UsageError_ {}

/// 解析命令行。
fn parse_options_(argv: impl IntoIterator<Item = String>) -> Result<Options, UsageError_> {
    let tokens: Vec<String> = argv.into_iter().collect();
    let mut listen = None;
    let mut kb_core = None;
    let mut runtime_dir = None;
    let mut storage_dir = None;

    let mut index = 0;
    while index < tokens.len() {
        match tokens[index].as_str() {
            "--listen" => {
                listen = Some(option_value_(&tokens, index, "--listen")?);
                index += 2;
            }
            "--kb-core" => {
                kb_core = Some(PathBuf::from(option_value_(&tokens, index, "--kb-core")?));
                index += 2;
            }
            "--runtime-dir" => {
                runtime_dir = Some(PathBuf::from(option_value_(
                    &tokens,
                    index,
                    "--runtime-dir",
                )?));
                index += 2;
            }
            "--storage-dir" => {
                storage_dir = Some(PathBuf::from(option_value_(
                    &tokens,
                    index,
                    "--storage-dir",
                )?));
                index += 2;
            }
            "-h" | "--help" => return Err(UsageError_("__help__".to_string())),
            other => return Err(UsageError_(format!("未知选项: {other}"))),
        }
    }

    let runtime_dir = runtime_dir.unwrap_or_else(default_runtime_dir_);
    let storage_dir = storage_dir.unwrap_or_else(|| runtime_dir.join("storage"));
    let kb_core = match kb_core.or_else(kb_core_starter::default_kb_core_path) {
        Some(path) => path,
        None => {
            return Err(UsageError_(
                "找不到 kb_core：请用 --kb-core <路径> 指定（缺省会在本程序同目录下找）"
                    .to_string(),
            ));
        }
    };

    Ok(Options {
        listen_: listen.unwrap_or_else(|| DEFAULT_LISTEN.to_string()),
        kb_core_: kb_core,
        runtime_dir_: runtime_dir,
        storage_dir_: storage_dir,
    })
}

/// 取 `tokens[index]` 后面那个取值。
fn option_value_(tokens: &[String], index: usize, name: &str) -> Result<String, UsageError_> {
    tokens
        .get(index + 1)
        .cloned()
        .ok_or_else(|| UsageError_(format!("选项 {name} 缺少取值")))
}

/// 与 `kb_core` 一致地推导运行时目录。
fn default_runtime_dir_() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(std::env::temp_dir)
        .join("llm_kb")
}

#[compio::main]
async fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let options = match parse_options_(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(error) if error.0 == "__help__" => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("参数错误: {error}");
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    warn_about_security(&options);

    match run(options).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// 把"没有鉴权"这件事在每次启动时都摆到台面上。
fn warn_about_security(options: &Options) {
    log::warn!(
        "本网关没有鉴权与 TLS：任何能访问 {} 的人都能读写知识库；仅用于受信网络",
        options.listen_
    );
}

/// 网关主体：启动 `kb_core`、连上它、然后一次服务一个远程客户端。
async fn run(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    // ── 系统层握手：启动 kb_core，拿到它的 IPC 端点文件名 ──────────────
    //
    // 这一步是**异步**的，而且与运行时无关：`kb_core_starter` 把"等 stdout 上
    // 那一行"收敛在自己的通知线程上，执行器线程不参与等待。网关现在不等超时
    // （等多久由调用方决定，将来要用 `--launch-timeout` 再说）；要加的话就是
    // `.may_cancel_with(超时令牌)`。
    let spec = LaunchSpec {
        kb_core: options.kb_core_.clone(),
        runtime_dir: options.runtime_dir_.clone(),
        storage_dir: options.storage_dir_.clone(),
    };
    let launched = start(&spec).await?;

    // ── 连上 kb_core（阻塞式连接，含重试）────────────────────────────
    let runtime_dir = options.runtime_dir_.clone();
    let client = compio::runtime::spawn_blocking(move || Client::connect(&runtime_dir))
        .await
        .map_err(|error| format!("连接任务异常结束: {error}"))??;

    // ── TCP 监听 ──────────────────────────────────────────────────────
    let listener = compio::net::TcpListener::bind(options.listen_.as_str()).await?;
    log::info!(
        "正在监听 {}（上游 kb_core pid={:?}，端点文件 {}）",
        options.listen_,
        launched.pid(),
        launched.name_file().display()
    );

    // 应用层握手**推迟到真有客户端连上来时**再做：启动阶段只保证"找得到"。
    let mut handshaken = false;

    loop {
        let (stream, peer) = listener.accept().await?;
        log::info!("远程客户端已连接: {peer}");

        if !handshaken {
            // 应用层握手：代表本网关自己跟 kb_core 谈一次，确认版本一致、
            // 上游真的能干活。远程客户端随后发来的 Hello 会被原样转发
            // （服务端会再答一次），这样"格式与 ipc 客户端相同"的性质不受影响。
            match client
                .hello(ClientInfo {
                    client_name: "kb_core_rproxy".to_string(),
                    client_version: env!("CARGO_PKG_VERSION").to_string(),
                    protocol_version: PROTOCOL_VERSION,
                })
                .await
            {
                Ok(info) => {
                    log::info!(
                        "应用层握手成功：服务端版本 {}，协议 v{}",
                        info.server_version,
                        info.protocol_version
                    );
                    handshaken = true;
                }
                Err(error) => {
                    log::error!("应用层握手失败，拒绝该客户端: {error}");
                    continue;
                }
            }
        }

        if let Err(error) = serve_one(stream, &client).await {
            log::warn!("本次连接结束: {error}");
        } else {
            log::info!("远程客户端已断开");
        }
    }
}

/// 一次读多少字节 TCP 数据。请求帧都很小，这个粒度够用。
const READ_CHUNK_BYTES_: usize = 4096;

/// 服务一个远程客户端，直到它断开。
///
/// 数据流：
///
/// ```text
/// TCP 读半 ──► 有界环形缓冲 ──► 帧解码 ──► IPC ──► 应答 ──► TCP 写半
///   feed_uplink              forward_uplink
/// ```
///
/// 两个任务都**借用**同一条流（`compio` 的 `split()` 给的是借用半部，不能 `spawn`），
/// 所以用 `zip` 并发跑在同一个任务里。
///
/// 收尾约定：转发侧无论因为什么收工，都会先发**中止信号**再返回——读侧可能正停等在
/// 满环上，而 buffex 的停等写者**不会**因为读端关闭而被释放（见 [`ring_`] 的模块文档）。
async fn serve_one(
    stream: compio::net::TcpStream,
    client: &Client,
) -> Result<(), Box<dyn std::error::Error>> {
    let (sink, source) = ring_::pipe(ring_::UPLINK_CAPACITY)
        .await
        .map_err(|error| std::io::Error::other(format!("建上行缓冲失败: {error}")))?;
    let (abort_tx, abort_rx) = futures_channel::oneshot::channel::<()>();

    let feeding = feed_uplink(&stream, sink, abort_rx);
    let forwarding = forward_uplink(&stream, source, client, abort_tx);

    let (_feeding, forwarding) = futures_lite::future::zip(feeding, forwarding).await;
    forwarding
}

/// 读侧：把 TCP 上的字节搬进有界环形缓冲（环满了就等 = 背压）。
///
/// 对端关闭写方向时置 EOF（[`ring_::close`]），让转发侧处理完残留再收工。
async fn feed_uplink(
    stream: &compio::net::TcpStream,
    mut sink: ring_::Sink,
    abort: futures_channel::oneshot::Receiver<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut reading_half, _writing_half) = stream.split();
    let mut chunk = vec![0u8; READ_CHUNK_BYTES_];

    let pumping = async {
        loop {
            let outcome = reading_half.read(chunk).await;
            let read = outcome.0?;
            chunk = outcome.1;

            if read == 0 {
                ring_::close(&mut sink);
                return Ok::<(), std::io::Error>(());
            }

            ring_::write_all(&mut sink, &chunk[..read])
                .await
                .map_err(|error| std::io::Error::other(error.to_string()))?;
        }
    };

    // 与"中止信号"赛跑：转发侧收工后读循环整体被丢弃，不会把整条连接拖住。
    let aborting = async {
        let _ = abort.await;
        Ok::<(), std::io::Error>(())
    };
    let _ = futures_lite::future::or(pumping, aborting).await;
    Ok(())
}

/// 转发侧：从环形缓冲里攒出完整帧、发给 `kb_core`、把应答原样写回 TCP。
///
/// 业务错误**不需要在这里特判**：它是 `Reply::Error` 这一正常应答，会被原样转发。
async fn forward_uplink(
    stream: &compio::net::TcpStream,
    mut source: ring_::Source,
    client: &Client,
    abort: futures_channel::oneshot::Sender<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (_reading_half, mut writing_half) = stream.split();
    let mut pending: Vec<u8> = Vec::new();

    let outcome = async {
        loop {
            if let Some(envelope) = frame_::decode_request(&mut pending)? {
                let request_id = envelope.request_id.clone();
                let reply = match client.send_envelope(envelope).await {
                    Ok(reply) => reply,
                    // `send_envelope` 原样回信封，理论上不会走业务错误分支；
                    // 真出现就按协议把它当成一条错误应答送回去。
                    Err(RpcError::Business(error)) => {
                        ReplyEnvelope::new(request_id, Reply::Error(error))
                    }
                    Err(RpcError::Transport(error)) => {
                        return Err(format!("向上游转发失败: {error}").into());
                    }
                };
                frame_::write_reply(&mut writing_half, &reply).await?;
                continue;
            }

            let Some(bytes) = ring_::read_some(&mut source, READ_CHUNK_BYTES_).await else {
                // 写端关闭且已读空：本次连接正常收工。
                return Ok(());
            };
            pending.extend_from_slice(&bytes);
        }
    }
    .await;

    // 无论如何都叫醒读侧（它可能正停等在满环上）。
    let _ = abort.send(());
    outcome
}

#[cfg(test)]
mod tests_ {
    use super::*;

    /// 把 `&str` 数组转成参数迭代器。
    fn argv_(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    /// 测试缺省值：监听 `0.0.0.0`（**不是** localhost）、数据目录在运行时目录下。
    ///
    /// - 手段：给一个显式的 `--kb-core` 与 `--runtime-dir`，其余用缺省。
    /// - 判断：`listen_` 是 `0.0.0.0:8788`（局域网测试的前提），
    ///   `storage_dir_` 落在运行时目录之下。
    #[test]
    fn defaults_bind_all_interfaces_() {
        let options = parse_options_(argv_(&[
            "--kb-core",
            "/usr/local/bin/kb-core",
            "--runtime-dir",
            "/run/kb",
        ]))
        .expect("应当能解析");

        assert_eq!(options.listen_, "0.0.0.0:8788");
        assert_eq!(options.runtime_dir_, PathBuf::from("/run/kb"));
        assert_eq!(options.storage_dir_, PathBuf::from("/run/kb/storage"));
    }

    /// 测试显式选项覆盖缺省值。
    ///
    /// - 手段：四个选项全给。
    /// - 判断：逐字段相等。
    #[test]
    fn explicit_options_win_() {
        let options = parse_options_(argv_(&[
            "--listen",
            "192.168.1.5:9000",
            "--kb-core",
            "/opt/kb-core",
            "--runtime-dir",
            "/tmp/rt",
            "--storage-dir",
            "/tmp/data",
        ]))
        .expect("应当能解析");

        assert_eq!(options.listen_, "192.168.1.5:9000");
        assert_eq!(options.kb_core_, PathBuf::from("/opt/kb-core"));
        assert_eq!(options.runtime_dir_, PathBuf::from("/tmp/rt"));
        assert_eq!(options.storage_dir_, PathBuf::from("/tmp/data"));
    }

    /// 测试未知选项与缺取值被拒绝。
    ///
    /// - 手段：分别给一个未知选项与一个没有取值的 `--listen`。
    /// - 判断：两者都返回错误，而不是被静默忽略。
    #[test]
    fn bad_options_are_rejected_() {
        assert!(parse_options_(argv_(&["--nope"])).is_err());
        assert!(parse_options_(argv_(&["--listen"])).is_err());
    }
}
