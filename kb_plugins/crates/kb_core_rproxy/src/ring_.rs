//! 有界字节管道：`buffex::circular_buff` 的 SPSC 环形缓冲。
//!
//! 网关用它给**上行**（远程客户端 → `kb_core`）加一个有界的中间缓冲：
//! TCP 读到的字节先进环，转发任务再从环里取。环满了写端就等，于是**读端不再
//! 从 socket 取数据**，TCP 窗口自然关上——这就是背压，而不是"内存无声涨上去"。
//!
//! # 三条来自实测的注意事项
//!
//! 这个库的停等语义有两个反直觉的地方（都在本机实测过）：
//!
//! 1. **只 drop 写端不等于 EOF**：必须显式 [`close`]，否则读者会永久停等；
//! 2. **满环上已经停等的写者，不会因为读端 `close`/drop 而被释放**。
//!    因此本模块的使用约定是：写者停等时，**唯一能救它的就是读者继续读**。
//!    网关侧的对应做法见 `main.rs`：转发任务出错退出前会先发一个中止信号，
//!    让读端任务整体被丢弃，而不是指望环形缓冲自己解开。
//! 3. `write_async` / `read_async` 返回的 future **不能交给 `tokio::spawn`**
//!    （rustc #100013）；本项目用 `compio`，实测可用。
//!
//! # 与 `MaybeUninit` 的关系
//!
//! 段（segment）的搬运是**位拷贝**，`u8` 是平凡类型，所以中间容器只能是
//! `MaybeUninit<u8>`。本模块把这点 `unsafe` 封在两个安全函数里，
//! 调用方看不到未初始化内存。

use core::mem::MaybeUninit;

use buffex::circular_buff::{
    BufConsumer, BufProducer, Consumer, CoreAlloc, Producer, SpscPair, builder::CircularBuffBuilder,
};
use buffex::x_deps::abs_buff::Demand;
use buffex::x_deps::abs_buff::buffer::TrBuffSegmRef;
use buffex::x_deps::mm_ptr::Owned;
use thiserror::Error;

/// 环形缓冲的类型。
///
/// **必须显式写出来**：builder 的类型参数在表达式位置推不出来
/// （不标注会报 `E0283`），所以它在库里是 `DefaultBuilder` 别名那样的存在。
pub type Buffer = Owned<[MaybeUninit<u8>], CoreAlloc>;

/// 写入端。
pub type Sink = Producer<BufConsumer<u8>, Buffer, u8, CoreAlloc>;

/// 读出端。
pub type Source = Consumer<BufProducer<u8>, Buffer, u8, CoreAlloc>;

/// 上行缓冲的容量（字节）。
///
/// 请求信封都很小（几百字节），64 KiB 足够吸收突发；调大只会推迟背压生效。
pub const UPLINK_CAPACITY: usize = 64 * 1024;

/// 环形缓冲的失败。
#[derive(Debug, Error)]
pub enum RingError {
    /// 创建失败（容量不合法等）。
    #[error("创建环形缓冲失败: {0}")]
    Build(String),

    /// 写入失败（写端已关闭）。
    #[error("写入环形缓冲失败: {0}")]
    Write(String),
}

/// 建一对有界字节管道。
///
/// # Errors
///
/// 容量不合法（小于 2 或超过 `(1 << 28) - 1`）时返回 [`RingError::Build`]。
pub async fn pipe(capacity: usize) -> Result<(Sink, Source), RingError> {
    let pair: SpscPair<Buffer> = CircularBuffBuilder::with_capacity(capacity)
        .map_err(|error| RingError::Build(format!("{error:?}")))?
        .build_async()
        .await
        .map_err(|error| RingError::Build(format!("{error:?}")))?;
    Ok(pair)
}

/// 把 `data` **全部**写进管道；环满了就等。
///
/// 这是背压的落点：等待期间调用方不会继续从 socket 取数据。
///
/// # Errors
///
/// 写端已关闭时返回 [`RingError::Write`]。
pub async fn write_all(sink: &mut Sink, mut data: &[u8]) -> Result<(), RingError> {
    while !data.is_empty() {
        let demand = Demand::at_least(1);
        let mut segment = match sink.write_async(&demand).await.pick_left() {
            Some(segment) => segment,
            None => {
                return Err(RingError::Write(
                    "写端已关闭（不再接受新的字节）".to_string(),
                ));
            }
        };

        let count = segment.least_count().min(data.len());
        let mut staged: Vec<MaybeUninit<u8>> = data[..count]
            .iter()
            .map(|byte| MaybeUninit::new(*byte))
            .collect();
        let moved = segment.move_items_from_buff(&mut staged);
        // 段 drop 时才真正提交并唤醒读者。
        drop(segment);

        data = &data[moved..];
        if moved == 0 {
            return Err(RingError::Write("写入没有进展".to_string()));
        }
    }
    Ok(())
}

/// 读一段字节。
///
/// - 管道为空**且写端仍开着**：等待，直到有数据（或写端关闭）；
/// - 写端已关闭且已读空：返回 `None`（EOF）。
///
/// 段操作本身不会失败（唯一的"另一种结果"就是上面那个 EOF），所以这个函数
/// 不返回 `Result`。
pub async fn read_some(source: &mut Source, max: usize) -> Option<Vec<u8>> {
    let demand = Demand::at_least(1);
    let mut segment = source.read_async(&demand).await.pick_left()?;

    let count = segment.least_count().min(max);
    let mut staged: Vec<MaybeUninit<u8>> = (0..count).map(|_| MaybeUninit::uninit()).collect();
    let moved = TrBuffSegmRef::move_items_to_buff(&mut segment, &mut staged);
    drop(segment);

    // SAFETY: `move_items_to_buff` 的返回值就是它写进 `staged` 的**已初始化**
    // 前缀长度；我们只读 `staged[..moved]`，因此这里的 `assume_init` 不会碰到
    // 未初始化的字节。
    let bytes = staged[..moved]
        .iter()
        .map(|cell| unsafe { cell.assume_init() })
        .collect();
    Some(bytes)
}

/// 关闭写端：置 EOF 并唤醒读者。
///
/// **只 drop 写端是不够的**——那样读者会一直等下去。
pub fn close(sink: &mut Sink) {
    sink.close();
}

#[cfg(test)]
mod tests_ {
    use super::*;

    /// 测试有序往返：写进去多少就读出来多少，且字节序一致。
    ///
    /// - 手段：建一个 64 字节的管道，写入 200 字节（超过容量，必然经历等待），
    ///   同时用 `compio::runtime::spawn` 起一个读者按 7 字节一段反复读。
    /// - 判断：读到的字节与写入的完全一致、总数相等——这同时验证了
    ///   "满环等待"与"读者继续读就释放写者"这条约定。
    #[compio::test]
    async fn bytes_round_trip_through_a_full_pipe_() {
        let (mut sink, mut source) = pipe(64).await.expect("应当能建管道");
        let payload: Vec<u8> = (0..200u32).map(|index| index as u8).collect();

        let expected = payload.clone();
        let reader = compio::runtime::spawn(async move {
            let mut collected = Vec::new();
            while collected.len() < expected.len() {
                let Some(chunk) = read_some(&mut source, 7).await else {
                    break;
                };
                collected.extend_from_slice(&chunk);
            }
            collected
        });

        write_all(&mut sink, &payload).await.expect("写应当成功");
        close(&mut sink);

        let collected = reader.await.expect("读者任务不应当异常结束");
        assert_eq!(collected, payload);
    }

    /// 测试关闭写端之后读者能读完残留，然后拿到 EOF。
    ///
    /// - 手段：写入 5 字节后就 `close`，然后连续 `read_some` 直到返回 `None`。
    /// - 判断：先读回全部 5 字节，再得到 `None`——`close` 真的置了 EOF
    ///   （这正是"只 drop 不 close 会永久停等"那条注意事项的反面）。
    #[compio::test]
    async fn close_marks_eof_after_remaining_bytes_() {
        let (mut sink, mut source) = pipe(16).await.expect("应当能建管道");
        write_all(&mut sink, &[1, 2, 3, 4, 5])
            .await
            .expect("写应当成功");
        close(&mut sink);

        let mut collected = Vec::new();
        while let Some(chunk) = read_some(&mut source, 8).await {
            collected.extend_from_slice(&chunk);
        }
        assert_eq!(collected, vec![1, 2, 3, 4, 5]);
    }

    /// 测试容量不合法会被拒绝，而不是 panic。
    ///
    /// - 手段：用容量 1（低于下限 2）建管道。
    /// - 判断：返回 `RingError::Build`。
    #[compio::test]
    async fn invalid_capacity_is_rejected_() {
        // 注意不能对 `outcome` 用 `{:?}`：成功时的 SPSC 两端没有实现 `Debug`。
        let outcome = pipe(1).await;
        assert!(
            matches!(outcome, Err(RingError::Build(_))),
            "容量 1 应当被拒绝"
        );
    }
}
