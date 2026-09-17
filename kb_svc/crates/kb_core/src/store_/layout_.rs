//! 存储布局：路径推导与标识校验。
//!
//! 布局本身在 [`super`] 的模块文档里描述；本文件只负责把它变成具体的
//! [`PathBuf`]，并把「哪些标识能安全地当文件名」这件事收在一处。
//!
//! 之所以把校验单独提出来，是因为标识来自客户端：协议里
//! `WorkspaceId` / `SessionId` 是**不透明字符串**，手工构造
//! （例如测试里写 `"w-1"`）是完全允许的，因此不能假定它一定由
//! `generate()` 产生。直接拿它拼路径会给出 `../` 穿越的机会。

use std::path::{Path, PathBuf};

use super::error_::StoreError;

/// 标识允许的最大字节数。
///
/// 定得比常见文件系统的单个文件名字节上限（255）保守得多，同时远大于
/// `<前缀>-<uuid-v4>` 的 34 字节。
const MAX_ID_BYTES_: usize = 128;

/// 全部工作区元数据所在目录：`<root>/workspaces`。
pub(super) fn workspaces_dir_(root: &Path) -> PathBuf {
    root.join("workspaces")
}

/// 某个工作区的元数据文件：`<root>/workspaces/<workspace_id>.json`。
pub(super) fn workspace_file_(root: &Path, workspace_id: &str) -> PathBuf {
    workspaces_dir_(root).join(format!("{workspace_id}.json"))
}

/// 全部会话的父目录：`<root>/sessions`。
pub(super) fn sessions_root_(root: &Path) -> PathBuf {
    root.join("sessions")
}

/// 某个工作区下的会话目录：`<root>/sessions/<workspace_id>`。
pub(super) fn sessions_dir_(root: &Path, workspace_id: &str) -> PathBuf {
    sessions_root_(root).join(workspace_id)
}

/// 某个会话的文件：`<root>/sessions/<workspace_id>/<session_id>.json`。
pub(super) fn session_file_(root: &Path, workspace_id: &str, session_id: &str) -> PathBuf {
    sessions_dir_(root, workspace_id).join(format!("{session_id}.json"))
}

/// 校验标识能否安全地当作文件名。
///
/// 允许的字符集是 ASCII 字母、数字、`-`、`_`；这与 `WorkspaceId::generate()`
/// 产生的形状（`<前缀>-<uuid-v4>`，全小写十六进制）一致，也覆盖测试里常用的
/// `"w-1"` 这类手写值。
///
/// **刻意收紧**而不是"只要没有 `/` 就行"：`abs_kb_svc` 的协议明确说标识是
/// 不透明字符串，而本阶段存储就是文件系统；把约束说清楚，比事后补一个
/// 路径穿越漏洞要好。
///
/// # Errors
///
/// 标识为空、超过 128 字节、或含允许集合之外的字符时返回
/// [`StoreError::InvalidId`]。
pub(super) fn check_id_(kind: &'static str, id: &str) -> Result<(), StoreError> {
    let allowed = !id.is_empty()
        && id.len() <= MAX_ID_BYTES_
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');

    if allowed {
        Ok(())
    } else {
        Err(StoreError::InvalidId {
            kind,
            id: id.to_string(),
        })
    }
}

#[cfg(test)]
mod tests_ {
    use super::*;

    /// 测试布局函数拼出的路径与文档中的目录树完全一致。
    ///
    /// - 手段：用固定的根目录与标识调用四个路径函数。
    /// - 判断：四个返回值逐字符等于手写的期望路径——布局一旦被改动，
    ///   本测试会先失败，从而逼着改动者同步更新文档与 README。
    #[test]
    fn layout_paths_match_documented_tree_() {
        let root = Path::new("/data/kb");

        assert_eq!(workspaces_dir_(root), Path::new("/data/kb/workspaces"));
        assert_eq!(
            workspace_file_(root, "w-1"),
            Path::new("/data/kb/workspaces/w-1.json")
        );
        assert_eq!(sessions_root_(root), Path::new("/data/kb/sessions"));
        assert_eq!(
            sessions_dir_(root, "w-1"),
            Path::new("/data/kb/sessions/w-1")
        );
        assert_eq!(
            session_file_(root, "w-1", "s-1"),
            Path::new("/data/kb/sessions/w-1/s-1.json")
        );
    }

    /// 测试标识校验接受生成器产物与手写短标识，拒绝可穿越路径的标识。
    ///
    /// - 手段：分别校验 `generate()` 的结果、`"w-1"`、空串、`"../escape"`、
    ///   `"a/b"`、超长串。
    /// - 判断：前两者通过；其余全部返回 [`StoreError::InvalidId`]，且错误里
    ///   保留了原始标识——这是"标识不可信"这条结论的直接体现。
    #[test]
    fn check_id_accepts_safe_and_rejects_path_like_() {
        use abs_kb_svc::v1::desktop::WorkspaceId;

        assert!(check_id_("工作区", WorkspaceId::generate().as_str()).is_ok());
        assert!(check_id_("工作区", "w-1").is_ok());
        assert!(check_id_("工作区", "w_1").is_ok());

        for bad in ["", "../escape", "a/b", "a\\b", ".hidden", "w 1", "工作区"] {
            let error = check_id_("工作区", bad).expect_err("应当被拒绝");
            assert!(
                matches!(&error, StoreError::InvalidId { id, .. } if id == bad),
                "实际错误: {error}"
            );
        }

        let long = "w".repeat(MAX_ID_BYTES_ + 1);
        assert!(check_id_("工作区", &long).is_err(), "超长标识应当被拒绝");
    }
}
