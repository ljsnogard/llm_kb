//! 反向用例：要求默认 `ipc::Service` 的订阅者满足 `Send`。
//!
//! 预期：编译失败。这决定了「kb_core 在 tokio 多线程运行时里持有 iceoryx2 端点」
//! 能否直接成立；若失败，就必须改用 `ipc_threadsafe::Service`，或者把端点固定在
//! 单线程上。

use iceoryx2::prelude::*;

fn assert_send<T: Send>() {}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    assert_send::<iceoryx2::port::subscriber::Subscriber<ipc::Service, u64, ()>>();
    assert_send::<iceoryx2::port::publisher::Publisher<ipc::Service, u64, ()>>();
    Ok(())
}
