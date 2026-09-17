//! 反向用例：载荷里带 `Option<u64>`。
//!
//! 预期：待定。现有 `wire.rs` 里大量使用 `Option<T>`（例如 `Usage` 的三个可选计数、
//! `Finished.reason`），这条用例用来确认 `Option` 能否直接进共享内存。

use iceoryx2::prelude::*;

#[repr(C)]
#[derive(Debug, Clone, Copy, ZeroCopySend)]
struct UsageLike {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let node = NodeBuilder::new().create::<ipc::Service>()?;
    let _service = node
        .service_builder(&"neg_option_payload".try_into()?)
        .publish_subscribe::<UsageLike>()
        .open_or_create()?;

    Ok(())
}
