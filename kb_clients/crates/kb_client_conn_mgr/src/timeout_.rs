//! 一个**与异步运行时无关**的"到点就取消"令牌。
//!
//! `abs_cancel` v0.2 只提供两种令牌：永不取消（`NonCancellableToken`）与
//! 一出生就已取消（`CancelledToken`）。而本 crate 需要的是第三种——**等一段
//! 时间之后取消**，用它实现"连不上就别一直等"。
//!
//! # 为什么放在客户端这里而不是上游
//!
//! 超时是**调用方的策略**：`kb_core_starter`、`kb_core_rproxy_wire` 都不该
//! 规定"等多久算超时"。本 crate 是那个"决定等多久"的调用方，所以定时器实现
//! 落在这里；将来若 `abs_cancel` 自己提供了超时令牌，这里可以直接换掉。
//!
//! # 形状
//!
//! 定时器是一条专职线程 + `oneshot`：`cancellation()` 返回的 future 只轮询
//! 完成量，**future 里没有阻塞**，因此 tokio / compio / `futures_lite::block_on`
//! 都能驱动它。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use abs_cancel::TrCancellationToken;
use futures_channel::oneshot;

/// 到点之后触发一次取消的令牌。
#[derive(Clone)]
pub struct TimeoutToken {
    /// 是否已经触发。
    fired_: Arc<AtomicBool>,

    /// 触发信号；`cancellation()` 被调用时把发送端存进来。
    sender_: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl TimeoutToken {
    /// 造一个 `after` 之后触发的令牌。
    ///
    /// 起一条专职线程睡到点再触发；`after` 为 0 或极小值时立即触发（线程仍然
    /// 起，只是马上结束）。线程名字固定为 `kb-client-timeout`，方便排错。
    pub fn after(after: Duration) -> Self {
        let token = Self {
            fired_: Arc::new(AtomicBool::new(false)),
            sender_: Arc::new(Mutex::new(None)),
        };

        let timer = token.clone();
        let spawned = std::thread::Builder::new()
            .name("kb-client-timeout".to_string())
            .spawn(move || {
                std::thread::sleep(after);
                timer.fire_();
            });
        if let Err(error) = spawned {
            // 起不了线程时**立即触发**：宁可"马上超时"也不要"永远不超时"。
            log::warn!("超时计时线程起不来，改为立即取消: {error}");
            token.fire_();
        }

        token
    }

    /// 触发取消（也可以手动调，测试里就这么用）。
    pub fn fire_(&self) {
        self.fired_.store(true, Ordering::SeqCst);
        let sender = self
            .sender_
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(sender) = sender {
            let _ = sender.send(());
        }
    }
}

impl TrCancellationToken for TimeoutToken {
    type Cancellation = oneshot::Receiver<()>;
    type ChildToken = TimeoutToken;

    fn is_cancelled(&self) -> bool {
        self.fired_.load(Ordering::SeqCst)
    }

    fn can_be_cancelled(&self) -> bool {
        true
    }

    fn child_token(&self) -> Self::ChildToken {
        self.clone()
    }

    fn cancellation(self) -> Self::Cancellation {
        let (sender, receiver) = oneshot::channel();
        if self.is_cancelled() {
            let _ = sender.send(());
            return receiver;
        }
        *self
            .sender_
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(sender);
        receiver
    }
}

#[cfg(test)]
mod tests_ {
    use super::*;
    use std::time::Instant;

    /// 测试"到点就取消"。
    ///
    /// - 手段：造一个 50ms 的超时令牌，先确认它没被取消，然后等它触发
    ///   （用 `cancellation()` 返回的 future 当完成量，配 `block_on`）。
    /// - 判断：`is_cancelled()` 一开始是 `false`；future 在远早于 10 秒的时间内
    ///   就绪；之后 `is_cancelled()` 是 `true`。
    #[test]
    fn fires_after_the_duration_() {
        let token = TimeoutToken::after(Duration::from_millis(50));
        assert!(!token.is_cancelled());

        let started = Instant::now();
        let wait = token.clone().cancellation();
        futures_lite::future::block_on(wait).expect("完成量应当被送到");
        assert!(started.elapsed() < Duration::from_secs(10));

        assert!(token.is_cancelled());
    }

    /// 测试手动触发与 `child_token` 共享同一个信号。
    ///
    /// - 手段：造一个很晚才到点的令牌（1 小时），立刻手动 `fire_()`；再取一个
    ///   `child_token()` 看它的状态。
    /// - 判断：本体与子令牌都立刻变成"已取消"，而且此时 `cancellation()` 直接
    ///   就绪——子令牌不是独立计时器，它跟本体是同一条信号。
    #[test]
    fn manual_fire_is_visible_to_child_tokens_() {
        let token = TimeoutToken::after(Duration::from_secs(3600));
        assert!(!token.is_cancelled());

        token.fire_();

        let child = token.child_token();
        assert!(token.is_cancelled());
        assert!(child.is_cancelled());
        assert!(child.can_be_cancelled());
        futures_lite::future::block_on(child.cancellation()).expect("应当立刻就绪");
    }
}
