//! 反向用例：把默认 `ipc::Service` 的订阅者移进另一个线程。
//!
//! 预期：待定。如果编译失败，说明默认 `ipc::Service` 的端点不是 `Send`，
//! 那么「在 tokio 里跑 kb_core 服务端」必须改用 `ipc_threadsafe::Service`，
//! 或者把 iceoryx2 的端点固定在单线程上。

use iceoryx2::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let node = NodeBuilder::new().create::<ipc::Service>()?;
    let service = node
        .service_builder(&"neg_move_across_thread".try_into()?)
        .publish_subscribe::<u64>()
        .open_or_create()?;
    let subscriber = service.subscriber_builder().create()?;

    let handle = std::thread::spawn(move || {
        let _ = subscriber.receive();
    });

    let _ = handle.join();
    Ok(())
}
