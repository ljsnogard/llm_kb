#![no_std]
#![feature(try_trait_v2)]

pub mod v1;

pub mod x_deps {
    pub use abs_async_iter;
    pub use abs_async_iter::x_deps::abs_cancel;
    pub use abs_str;
    pub use anylr;
}
