//! 反向用例：用 `String` 当 pub/sub 载荷。
//!
//! 预期：编译失败，提示 `String: ZeroCopySend` 不成立。

use iceoryx2::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let node = NodeBuilder::new().create::<ipc::Service>()?;
    let _service = node
        .service_builder(&"neg_string_payload".try_into()?)
        .publish_subscribe::<String>()
        .open_or_create()?;

    Ok(())
}
