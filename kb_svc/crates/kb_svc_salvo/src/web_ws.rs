//! 浏览器侧 WebSocket 会话：把浏览器消息交给 [`AppState`]，把广播事件推回页面。

use std::sync::Arc;

use salvo::websocket::{Message, WebSocket};
use tokio::sync::broadcast;

use crate::{
    hub::AppState,
    wire::{ClientMessage, ErrorCode, ServerMessage},
};

/// 驱动一个浏览器连接，直到它断开。
///
/// # 并发模型
///
/// - **下行**：一条转发任务订阅 [`AppState`] 的广播流，把事件编码成 JSON 文本帧；
/// - **上行**：主循环直接读取浏览器的请求帧并同步处理。
///
/// 之所以不把上行也拆成任务，是因为 `WebSocket` 本身不是 `Sync`，拆开需要额外的
/// 同步开销，而处理上行消息是纯 CPU 的轻量操作。
pub async fn serve_chat(
    mut socket: WebSocket,
    state: Arc<AppState>,
    mut events: broadcast::Receiver<ServerMessage>,
) {
    // 先发一条 ready，让界面立刻知道插件是否在线、当前用哪个服务。
    let ready = state.ready_message().await;
    if !send_json(&mut socket, &ready).await {
        return;
    }

    // 新页面加入时，把「当前是否有插件」的变化也广播给其它页面没有意义，
    // 因此 ready 只发给本连接。

    loop {
        tokio::select! {
            // 下行：广播事件 → 浏览器
            received = events.recv() => {
                match received {
                    Ok(message) => {
                        if !send_json(&mut socket, &message).await {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        log::warn!("浏览器订阅落后，丢弃了 {skipped} 条事件");
                        let notice = ServerMessage::Error {
                            turn_id: None,
                            code: ErrorCode::Internal,
                            message: format!("事件积压，已丢弃 {skipped} 条增量，界面可能不完整"),
                        };
                        if !send_json(&mut socket, &notice).await {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }

            // 上行：浏览器 → 服务端
            received = socket.recv() => {
                let Some(frame) = received else { break };
                let frame = match frame {
                    Ok(frame) => frame,
                    Err(err) => {
                        log::debug!("浏览器连接读取失败: {err}");
                        break;
                    }
                };

                if frame.is_close() {
                    break;
                }

                let Ok(text) = frame.as_str() else {
                    // 二进制帧在当前协议里没有定义用途，忽略但记录。
                    log::debug!("忽略非文本帧: {} 字节", frame.as_bytes().len());
                    continue;
                };

                let message: ClientMessage = match serde_json::from_str(text) {
                    Ok(message) => message,
                    Err(err) => {
                        let notice = ServerMessage::Error {
                            turn_id: None,
                            code: ErrorCode::BadRequest,
                            message: format!("无法解析请求: {err}"),
                        };
                        if !send_json(&mut socket, &notice).await {
                            break;
                        }
                        continue;
                    }
                };

                // `UseService` 成功后也要让所有页面知道生效服务变了，因此在
                // 广播通道上再发一条 ready 供界面刷新。
                let is_use_service = matches!(message, ClientMessage::UseService { .. });

                if let Err(err) = state.handle_client_message(message).await {
                    let notice = err.into_message();
                    if !send_json(&mut socket, &notice).await {
                        break;
                    }
                } else if is_use_service {
                    let ready = state.ready_message().await;
                    state.broadcast(ready);
                }
            }
        }
    }
}

/// 把一条消息序列化成 JSON 文本帧发出；返回 `false` 表示连接已不可用。
async fn send_json<T: serde::Serialize>(socket: &mut WebSocket, message: &T) -> bool {
    let text = match serde_json::to_string(message) {
        Ok(text) => text,
        Err(err) => {
            log::error!("序列化下行消息失败: {err}");
            return true;
        }
    };

    match socket.send(Message::text(text)).await {
        Ok(()) => true,
        Err(err) => {
            log::debug!("浏览器连接写出失败: {err}");
            false
        }
    }
}
