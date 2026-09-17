//! 反向用例：把 `String` 嵌进 `#[repr(C)]` 结构体并派生 `ZeroCopySend`。
//!
//! 预期：编译失败。这条用例最贴近「把 `abs_llm::v1` / `wire.rs` 的类型直接搬上共享内存」
//! 的设想，用来验证该设想是否可行。

use iceoryx2::prelude::*;

#[repr(C)]
#[derive(ZeroCopySend)]
struct LooksLikeWireType {
    turn_id: String,
    text: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let node = NodeBuilder::new().create::<ipc::Service>()?;
    let _service = node
        .service_builder(&"neg_nested_string".try_into()?)
        .publish_subscribe::<LooksLikeWireType>()
        .open_or_create()?;

    Ok(())
}
