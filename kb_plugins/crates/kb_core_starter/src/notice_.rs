//! 解析 `kb_core` 在 stdout 上打的那**一行 JSON 通知**。
//!
//! 通知的**格式由 `kb_core` 决定**（`--handshake-prompt stdio` 时才有），
//! **不属于 `abs_kb_svc` 的协议**——它是传输/启动层约定，换实现可以改：
//!
//! ```json
//! {"event":"ipc_ready","ipc_name_file":"…/kb-20260918-….ipc","protocol_version":1,"pid":1234}
//! ```
//!
//! 本模块只关心 `ipc_name_file`。它承诺的是**文件名**，不是"现在就能连上"：
//! 文件内容（当前可连的端点名）要等 `kb_core` 真正开始 accept 才会写进去，
//! 所以连接方的重试是必需的。

use std::path::PathBuf;

use crate::error_::LaunchError;

/// 通知里携带端点文件名的字段名。
const NAME_FILE_FIELD_: &str = "ipc_name_file";

/// 从一行通知里取出 `ipc_name_file`。
///
/// 前后空白容忍（stdout 那一行带换行）。字段缺失、不是字符串、或者取值为空，
/// 一律算 [`LaunchError::MissingField`]——空路径不可能是有效的端点文件名。
///
/// # Errors
///
/// - [`LaunchError::BadNotice`]：不是合法 JSON；
/// - [`LaunchError::MissingField`]：没有可用的 `ipc_name_file`。
pub(super) fn parse_name_file_(line: &str) -> Result<PathBuf, LaunchError> {
    let notice: serde_json::Value =
        serde_json::from_str(line.trim()).map_err(LaunchError::BadNotice)?;

    let name_file = notice
        .get(NAME_FILE_FIELD_)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if name_file.trim().is_empty() {
        return Err(LaunchError::MissingField(NAME_FILE_FIELD_));
    }

    Ok(PathBuf::from(name_file))
}

#[cfg(test)]
mod tests_ {
    use super::*;

    /// 测试合法通知能取出端点文件名。
    ///
    /// - 手段：喂一行 `kb_core` 实际会打的完整通知（四个字段齐全，末尾带换行）。
    /// - 判断：返回的路径与 `ipc_name_file` 完全一致——多出来的字段不影响解析，
    ///   末尾换行也不会被算进路径。
    #[test]
    fn parses_name_file_from_a_full_notice_() {
        let line = "{\"event\":\"ipc_ready\",\
                    \"ipc_name_file\":\"/run/user/1000/llm_kb/kb-20260918-abc.ipc\",\
                    \"protocol_version\":1,\"pid\":1234}\n";

        let name_file = parse_name_file_(line).expect("应当能解析");

        assert_eq!(
            name_file,
            PathBuf::from("/run/user/1000/llm_kb/kb-20260918-abc.ipc")
        );
    }

    /// 测试坏通知会被明确拒绝，而不是被当成合法路径放过去。
    ///
    /// - 手段：分别喂"不是 JSON"、"缺 `ipc_name_file` 字段"、"该字段是空串"三种输入。
    /// - 判断：前两种分别得到 [`LaunchError::BadNotice`] 与
    ///   [`LaunchError::MissingField`]；空串也必须是 `MissingField`——
    ///   空路径若被放过去，调用方会拿着一个不存在的名字文件去重试到超时。
    #[test]
    fn rejects_bad_notices_() {
        assert!(matches!(
            parse_name_file_("你好，不是 JSON"),
            Err(LaunchError::BadNotice(_))
        ));

        assert!(matches!(
            parse_name_file_("{\"event\":\"ipc_ready\",\"pid\":1}"),
            Err(LaunchError::MissingField("ipc_name_file"))
        ));

        assert!(matches!(
            parse_name_file_("{\"ipc_name_file\":\"   \"}"),
            Err(LaunchError::MissingField("ipc_name_file"))
        ));
    }
}
