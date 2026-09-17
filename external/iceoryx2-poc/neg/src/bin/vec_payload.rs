//! 反向用例：用 `Vec<u8>` 当 pub/sub 载荷。
//!
//! 预期：编译失败，提示 `Vec<u8>: ZeroCopySend` 不成立。

use iceoryx2::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let node = NodeBuilder::new().create::<ipc::Service>()?;
    let _service = node
        .service_builder(&"neg_vec_payload".try_into()?)
        .publish_subscribe::<Vec<u8>>()
        .open_or_create()?;

    Ok(())
}
