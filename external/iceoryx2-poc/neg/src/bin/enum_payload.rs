//! 反向用例：`#[repr(C)]` 枚举 + `ZeroCopySend` 派生是否被支持。
//!
//! 预期：待定——如果编译通过，说明枚举可用作载荷；如果失败，说明载荷必须是
//! 「无 niche 的普通结构体」，现有 `wire.rs` 里那种带 tag 的枚举形状需要改造。
//!
//! `Debug` 是 iceoryx2 载荷的硬性要求（`Payload: Debug`），这里显式派生，
//! 免得枚举本身的结论被 `Debug` 缺失掩盖。

use iceoryx2::prelude::*;

#[repr(C)]
#[derive(Debug, Clone, Copy, ZeroCopySend)]
enum Command {
    Ping,
    Ask { turn_id: [u8; 36], len: u32 },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let node = NodeBuilder::new().create::<ipc::Service>()?;
    let _service = node
        .service_builder(&"neg_enum_payload".try_into()?)
        .publish_subscribe::<Command>()
        .open_or_create()?;

    Ok(())
}
