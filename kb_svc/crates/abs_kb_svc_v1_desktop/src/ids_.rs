//! 标识类型：谁生成、长什么样、以及"本地先创建"如何过渡到"服务端分配"。
//!
//! # 谁生成什么
//!
//! | 标识 | 生成方 | 为什么 |
//! | :--- | :--- | :--- |
//! | [`WorkspaceId`] | **`kb_core`** | 工作区由 `kb_core` 持有并多端同步，标识必须全局唯一且由它分配 |
//! | [`SessionId`] | **`kb_core`** | 会话同样归 `kb_core` 管理 |
//! | [`TurnId`] | 客户端 | 一轮生成由发起方生成，服务端据此回报增量；这样"提问"到"第一条增量"之间也有标识可用 |
//! | [`LocalId`] | 客户端 | **本地先创建、尚未同步**的对象在客户端的临时标识 |
//! | [`ServiceId`] | 用户 | LLM 服务的名字，是用户可见的标识，不是 uuid |
//! | [`RequestId`] | 客户端 | 每次连接内唯一，用于把应答配回请求 |
//!
//! # 格式约定
//!
//! 由 `kb_core` 或客户端**生成**的标识形如 `<前缀>-<uuid-v4>`，例如：
//!
//! ```text
//! w-3f2b9c1d4e5a4b7c8d9e0f1a2b3c4d5e    工作区
//! s-9a1c77e0b2d34f5a8c6e0b1d2f3a4c5b    会话
//! t-0d5e8a2b4c6f4a1b9e7d3c5a2f8b6e4d    一轮生成
//! l-7c1f3a5b9d2e4f6a8b0c2d4e6f8a0b1c    本地临时标识
//! q-1b2d3f4a5c6e7b8d9f0a1c2e3b4d5f6a    请求
//! ```
//!
//! 前缀只是**便于人读**：接收方不应解析它，标识一律当作不透明字符串处理。
//! 生成函数（[`WorkspaceId::generate`] 等）保证约定一致；手工构造
//! （例如测试里写 `"w-1"`）也完全允许。
//!
//! # 本地先创建、同步时再分配标识
//!
//! 客户端可以在**还没有连上 `kb_core`** 的情况下先本地创建工作区与会话，
//! 此时它只能拿到一个 [`LocalId`]：
//!
//! ```text
//!   客户端本地                               kb_core
//!   ─────────                              ────────
//!   新建工作区 → LocalId("l-…")             （尚不存在）
//!        └── AddWorkspace { local_id, name, path } ──►  分配 WorkspaceId
//!        ◄── WorkspaceAdded { local_id, workspace } ──  并持久化
//!   把 LocalId 换成 WorkspaceId，此后一律用服务端标识
//! ```
//!
//! 同理，会话通过 [`CreateSession`](crate::Request::CreateSession)
//! 同步时由 `kb_core` 分配 [`SessionId`]。**`LocalId` 只在同步之前有意义**：
//! 同步完成（或该对象被丢弃）之后不应当再出现在任何消息里。
//!
//! 之所以用**两个不同的类型**而不是"一个可以为空的 id"，是为了让"还没同步"
//! 这件事在类型层面就不可忽略——把 `LocalId` 当成 `WorkspaceId` 用是编译错误。

use serde::{Deserialize, Serialize};

/// 声明一个「字符串形式的新类型标识」。
///
/// 这是本模块**唯一的共享私有逻辑**，因此把所有标识都收在这一个文件里。
/// 两副分支：带前缀的额外得到 `generate()` 与 `PREFIX`；不带的只有基本能力。
///
/// 该宏只在**本模块内**可用（未导出），因此无需对外文档。
macro_rules! string_id {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        string_id!($(#[$doc])* $name);

        impl $name {
            /// 本类型标识的前缀（仅用于人类阅读，不参与解析）。
            pub const PREFIX: &'static str = $prefix;

            /// 按 `<前缀>-<uuid-v4>` 生成一个新的标识。
            ///
            /// # Examples
            ///
            /// ```
            #[doc = concat!("use abs_kb_svc_v1_desktop::", stringify!($name), ";")]
            ///
            #[doc = concat!("let id = ", stringify!($name), "::generate();")]
            #[doc = concat!("assert!(id.as_str().starts_with(\"", $prefix, "-\"));")]
            /// ```
            pub fn generate() -> Self {
                Self(format!("{}-{}", $prefix, uuid::Uuid::new_v4().simple()))
            }
        }
    };

    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// 由字符串构造。
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// 取得内部字符串。
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// 取出内部字符串。
            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(
    /// 工作区标识，由 `kb_core` 分配。
    ///
    /// 客户端在本地先创建工作区时还拿不到它，那时用 [`LocalId`]。
    WorkspaceId,
    "w"
);

string_id!(
    /// 会话标识，由 `kb_core` 分配。
    ///
    /// 客户端在本地先创建会话时还拿不到它，那时用 [`LocalId`]。
    SessionId,
    "s"
);

string_id!(
    /// 一轮生成（一问一答）的标识，由**客户端**生成。
    TurnId,
    "t"
);

string_id!(
    /// 客户端本地的临时标识。
    ///
    /// 用于"已经本地创建、但还没同步到 `kb_core`"的对象。它只在发起它的客户端
    /// 内部有意义，同步完成后应当被 `kb_core` 分配的标识取代。
    LocalId,
    "l"
);

string_id!(
    /// 请求标识，由客户端生成，在本次连接内唯一。
    RequestId,
    "q"
);

string_id!(
    /// LLM 服务标识：用户可见的名字（例如 `deepseek`），不是 uuid。
    ///
    /// 它由用户在设置里取名，因此**没有** `generate()`——不需要 uuid 生成规则。
    /// 之所以仍然是新类型，是为了不和其它字符串标识混用。
    ServiceId
);

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试生成的标识带正确前缀，且每次都不同。
    ///
    /// - 手段：用 `WorkspaceId::generate()` 生成 128 个标识。
    /// - 判断：每个都以 `w-` 开头，且放进 `HashSet` 后仍为 128 个——
    ///   说明 uuid 提供了足够的唯一性，也说明格式约定被统一实现在一处。
    #[test]
    fn generated_ids_carry_prefix_and_are_unique_() {
        use std::collections::HashSet;

        let ids: HashSet<WorkspaceId> = (0..128).map(|_| WorkspaceId::generate()).collect();
        assert_eq!(ids.len(), 128, "生成的标识不应重复");

        for id in &ids {
            assert!(id.as_str().starts_with("w-"), "实际标识: {id}");
        }
    }

    /// 测试各标识的新类型不会互相混淆（类型层面）。
    ///
    /// - 手段：分别构造 `WorkspaceId` / `SessionId` / `TurnId` / `LocalId` 并比较其字符串。
    /// - 判断：四者字符串可以相同，但字段类型不同——本测试只断言 `as_str` /
    ///   `From<&str>` 的表现一致，真正的隔离由编译器保证（见上一个测试的注释）。
    #[test]
    fn id_newtypes_expose_plain_strings_() {
        let workspace = WorkspaceId::new("w-1");
        let local: LocalId = "w-1".into();

        assert_eq!(workspace.as_str(), "w-1");
        assert_eq!(local.as_str(), "w-1");
        assert_eq!(local.to_string(), "w-1");
        assert_eq!(local.clone().into_string(), "w-1");
    }

    /// 测试带前缀的标识在序列化后是裸字符串（`#[serde(transparent)]` 生效）。
    ///
    /// - 手段：序列化 `TurnId` 与 `LocalId`。
    /// - 判断：结果都是 `"…"` 形式的裸字符串，而不是 `{"0":…}` 这样的包装对象，
    ///   客户端无需为标识做额外解包。
    #[test]
    fn ids_serialize_transparently_() {
        assert_eq!(
            serde_json::to_string(&TurnId::new("t-1")).expect("应当能序列化"),
            r#""t-1""#
        );
        assert_eq!(
            serde_json::to_string(&LocalId::new("l-1")).expect("应当能序列化"),
            r#""l-1""#
        );
    }
}
