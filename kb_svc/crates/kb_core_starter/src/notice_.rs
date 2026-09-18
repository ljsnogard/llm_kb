//! 解析 `kb_core` 在 stdout 上打的那**一行 JSON 通知**。
//!
//! 通知的**类型是 [`IpcReadyNotice`]**——它属于**公开协议**（定义在
//! `abs_kb_core_handshake`），不是本 crate 或 `kb_core` 私定的格式。
//! 发的一方（`kb_core::serve_`）与收的一方（本模块）共用同一份字段定义，
//! 因此字段名/取值一改就是编译错误，而不是"跑起来才发现对端解不开"。
//!
//! 本模块只从里面取 `ipc_name_file`。它承诺的是**文件名**，不是"现在就能连上"：
//! 文件内容（当前可连的端点名）要等 `kb_core` 真正开始 accept 才会写进去，
//! 所以连接方的重试是必需的。

use std::path::PathBuf;

use abs_kb_core_handshake::IpcReadyNotice;

use crate::error_::LaunchError;

/// 通知里携带端点文件名的字段名（只用于错误信息；字段本身由协议类型保证）。
const NAME_FILE_FIELD_: &str = "ipc_name_file";

/// 从一行通知里取出 `ipc_name_file`。
///
/// 前后空白容忍（stdout 那一行带换行）。取值是空串时报
/// [`LaunchError::MissingField`]——空路径不可能是有效的端点文件名，放过去只会让
/// 调用方拿着一个不存在的名字文件重试到超时。
///
/// # Errors
///
/// - [`LaunchError::BadNotice`]：不是合法 JSON、缺字段、或者 `event` 不是
///   [`IpcReadyNotice`] 认得的取值（`"ipc_ready"`）；
/// - [`LaunchError::MissingField`]：`ipc_name_file` 是空串。
pub(super) fn parse_name_file_(line: &str) -> Result<PathBuf, LaunchError> {
    let notice: IpcReadyNotice =
        serde_json::from_str(line.trim()).map_err(LaunchError::BadNotice)?;

    if notice.ipc_name_file.trim().is_empty() {
        return Err(LaunchError::MissingField(NAME_FILE_FIELD_));
    }

    Ok(PathBuf::from(notice.ipc_name_file))
}

#[cfg(test)]
mod tests_ {
    use super::*;

    /// 测试合法通知能取出端点文件名。
    ///
    /// - 手段：喂一行 `kb_core` 实际会打的完整通知（四个字段齐全，末尾带换行）。
    /// - 判断：返回的路径与 `ipc_name_file` 完全一致——末尾换行不会被算进路径。
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
    /// - 手段：分别喂"不是 JSON"、"缺 `ipc_name_file`"、"`event` 取值不认识"
    ///   （用 Rust 枚举名 `IpcReady` 冒充线上取值）、"`ipc_name_file` 是空串"。
    /// - 判断：前三者都是 [`LaunchError::BadNotice`]（类型化解析在协议边界上把关，
    ///   拼错字段名或枚举名不会"尽力而为"地通过）；空串是
    ///   [`LaunchError::MissingField`]——空路径若被放过去，调用方会拿着一个不存在的
    ///   名字文件重试到超时。
    #[test]
    fn rejects_bad_notices_() {
        assert!(matches!(
            parse_name_file_("你好，不是 JSON"),
            Err(LaunchError::BadNotice(_))
        ));

        assert!(matches!(
            parse_name_file_("{\"event\":\"ipc_ready\",\"protocol_version\":1,\"pid\":1}"),
            Err(LaunchError::BadNotice(_))
        ));

        assert!(matches!(
            parse_name_file_(
                "{\"event\":\"IpcReady\",\"ipc_name_file\":\"/tmp/x.ipc\",\
                 \"protocol_version\":1,\"pid\":1}"
            ),
            Err(LaunchError::BadNotice(_))
        ));

        assert!(matches!(
            parse_name_file_(
                "{\"event\":\"ipc_ready\",\"ipc_name_file\":\"   \",\
                 \"protocol_version\":1,\"pid\":1}"
            ),
            Err(LaunchError::MissingField("ipc_name_file"))
        ));
    }
}
