//! 工作区目录浏览。
//!
//! 对应桌面客户端右侧的「工作区文件」面板。界面骨架已经在客户端里立好了，
//! 只等这条通道接上。

use serde::{Deserialize, Serialize};

use super::ids_::WorkspaceId;

/// 目录条目的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirEntryKind {
    /// 普通文件。
    File,

    /// 目录。
    Directory,

    /// 其它（符号链接、设备等）。
    Other,
}

/// 目录里的一个条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    /// 条目名（不含父路径）。
    pub name: String,

    /// 条目类型。
    pub kind: DirEntryKind,

    /// 文件大小时为 `Some`；目录通常为 `None`。
    #[serde(default)]
    pub size_bytes: Option<u64>,
}

/// 一次目录列举的结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryListing {
    /// 所属工作区。
    pub workspace_id: WorkspaceId,

    /// 相对于工作区根目录的路径；根目录用空串表示。
    pub relative_path: String,

    /// 该目录下的条目。
    pub entries: Vec<DirEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试目录列举结果能往返序列化，且条目类型为 snake_case。
    ///
    /// - 手段：构造含文件与目录两种条目的 `DirectoryListing`，序列化后反序列化。
    /// - 判断：JSON 中出现 `"kind":"file"` 与 `"kind":"directory"`；
    ///   文件条目 `size_bytes` 为 1024，目录条目为 `null`（不省略）。
    #[test]
    fn directory_listing_round_trips_() {
        let listing = DirectoryListing {
            workspace_id: WorkspaceId::new("w-1"),
            relative_path: "notes".to_string(),
            entries: vec![
                DirEntry {
                    name: "a.md".to_string(),
                    kind: DirEntryKind::File,
                    size_bytes: Some(1024),
                },
                DirEntry {
                    name: "sub".to_string(),
                    kind: DirEntryKind::Directory,
                    size_bytes: None,
                },
            ],
        };

        let json = serde_json::to_string(&listing).expect("应当能序列化");
        assert!(json.contains(r#""kind":"file""#), "实际 JSON: {json}");
        assert!(json.contains(r#""kind":"directory""#), "实际 JSON: {json}");
        assert!(json.contains(r#""size_bytes":1024"#), "实际 JSON: {json}");
        assert!(json.contains(r#""size_bytes":null"#), "实际 JSON: {json}");

        let parsed: DirectoryListing = serde_json::from_str(&json).expect("应当能反序列化");
        assert_eq!(parsed, listing);
    }
}
