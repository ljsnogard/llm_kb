//! 线程安全 × 异步运行时的可用性验证。
//!
//! 关注点：`ipc::Service` 的端口是否 `Send`/`Sync`（能否丢进 `tokio::spawn` 或
//! `std::thread`），以及 `ipc_threadsafe::Service` 是否解决这一点。
//!
//! 注意：`ipc::Service` 的「不是 Send」这一点由反向用例
//! `neg/src/bin/send_required.rs` 证明，这里只验证 `ipc_threadsafe::Service` 可用。

use std::time::Duration;

use iceoryx2::prelude::*;

fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let node = NodeBuilder::new().create::<ipc_threadsafe::Service>()?;
    let service = node
        .service_builder(&"llm_kb_poc_threadsafe".try_into()?)
        .publish_subscribe::<u64>()
        .open_or_create()?;

    let subscriber = service.subscriber_builder().create()?;
    let publisher = service.publisher_builder().create()?;

    assert_send::<iceoryx2::port::subscriber::Subscriber<ipc_threadsafe::Service, u64, ()>>();
    assert_sync::<iceoryx2::port::subscriber::Subscriber<ipc_threadsafe::Service, u64, ()>>();
    assert_send::<iceoryx2::port::publisher::Publisher<ipc_threadsafe::Service, u64, ()>>();
    assert_sync::<iceoryx2::port::publisher::Publisher<ipc_threadsafe::Service, u64, ()>>();

    // 先把一条数据发出去，避免订阅线程空转超时。
    let sample = publisher.loan_uninit()?;
    let sample = sample.write_payload(42u64);
    sample.send()?;

    // 把订阅者丢进独立线程：这就是 tokio::spawn 所需的同一类约束（Send + 'static）。
    let handle = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_millis(2000);
        while std::time::Instant::now() < deadline {
            match subscriber.receive() {
                Ok(Some(value)) => return format!("received {}", *value),
                Ok(None) => std::thread::sleep(Duration::from_millis(1)),
                Err(err) => return format!("error {err:?}"),
            }
        }
        "timeout".to_string()
    });

    println!(
        "THREADSAFE-DONE {}",
        handle.join().unwrap_or_else(|_| "panic".to_string())
    );

    Ok(())
}
