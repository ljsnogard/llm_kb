//! # abs_kb_svc
//!
//! 知识库服务的 **业务通信抽象层**：定义各插件与 `kb_core` 通信的抽象代码接口、
//! 通信内容，以及一个**与异步运行时无关的异步 RPC 业务接口**。
//!
//! # 它不是什么
//!
//! 它不依赖任何具体传输（ipc-channel / socket / 同进程线程），也不定义"数据在
//! 线上长什么样"——那是实现 crate（`kb_svc_servo_ipc` 等）的职责。
//!
//! # 模块地图
//!
//! | 模块 | 职责 |
//! | :--- | :--- |
//! | [`v1`] | 协议 v1 的全部约定 |
//! | [`v1::desktop`] | 桌面客户端 `kb_admin_desktop` 与 `kb_core` 交换的数据 |
//!
//! # 设计约定
//!
//! - **与运行时无关**：公开 API 里不出现 tokio / async-std / compio 的类型；
//!   异步接口用 `abs_cancel::TrMayCancel` 表达，异步迭代用
//!   `abs_async_iter::{TrAsyncIterator, TrFlux}` 表达。
//! - **future 里禁止阻塞**：阻塞式传输必须把等待收敛到自己的 IO 线程上，
//!   详见 crate 根目录的 `README.md`。
//! - **错误分层**：传输层错误走实现 crate 的错误类型；业务错误是协议里的一种应答。
//!
//! # 当前状态
//!
//! [`v1::desktop`] 已给出桌面客户端侧的数据定义；抽象的异步 RPC trait 尚未落地。
//! 背景与待决策项见 `dev-notes/`。

pub mod v1;
