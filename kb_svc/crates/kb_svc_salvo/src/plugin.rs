//! 插件侧 WebSocket 路由：接收 `kb_rig_llm` 的连接，并在其上做双向转发。
//!
//! 插件由 `kb_core` 作为子进程启动，启动时通过 `--socket <path>` 与
//! `LLM_KB_PLUGIN_SOCKET` 得知本服务的 Unix domain socket 路径，随后连接
//! `ws://localhost/ws/plugin`。
//!
//! # 转发方向
//!
//! - 下行：`AppState` 里的指令队列（`ask` / `cancel` / `ping`）→ 插件；
//! - 上行：插件事件（`started` / `delta` / `usage` / `finished` / `error`）→
//!   交给 `AppState` 投影成浏览器事件并广播。

use std::sync::Arc;

use salvo::websocket::{Message, WebSocket, WebSocketUpgrade};
use salvo::{Depot, Request, Response, Router, http::StatusCode, prelude::*};
use uuid::Uuid;

use crate::{hub::AppState, wire::PluginRequest};

/// 构造插件侧的路由。
pub fn router() -> Router {
    Router::with_path("ws/plugin").goal(plugin_ws)
}

/// 从 `Depot` 取出共享状态。
fn state(depot: &Depot) -> Result<Arc<AppState>, StatusError> {
    depot
        .get_typed::<Arc<AppState>>()
        .cloned()
        .map_err(|_| StatusError::internal_server_error().brief("AppState 未注入"))
}

/// `GET /ws/plugin`：插件连接入口。
#[handler]
async fn plugin_ws(req: &mut Request, res: &mut Response, depot: &mut Depot) {
    let Ok(state) = state(depot) else {
        res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
        return;
    };

    let upgrade = WebSocketUpgrade::new();
    let result = upgrade
        .upgrade(req, res, move |socket| async move {
            serve_plugin(socket, state).await;
        })
        .await;

    if let Err(err) = result {
        log::warn!("插件 WebSocket 升级失败: {err}");
    }
}

/// 驱动一个插件连接，直到它断开。
async fn serve_plugin(mut socket: WebSocket, state: Arc<AppState>) {
    let connection_id = Uuid::new_v4();
    let mut requests = state.register_plugin(connection_id);

    log::info!("插件已连接: {connection_id}");

    // 连接建立后主动问一声，方便插件（重新）上报自己的能力。
    let _ = socket
        .send(Message::text(
            serde_json::to_string(&PluginRequest::Hello)
                .unwrap_or_else(|_| "{\"type\":\"hello\"}".to_string()),
        ))
        .await;

    // 让浏览器知道插件上线了。
    state.broadcast(state.ready_message().await);

    loop {
        tokio::select! {
            // 下行：把 hub 里的指令写给插件。
            request = requests.recv() => {
                let Some(request) = request else { break };

                let text = match serde_json::to_string(&request) {
                    Ok(text) => text,
                    Err(err) => {
                        log::error!("序列化插件指令失败: {err}");
                        continue;
                    }
                };

                if let Err(err) = socket.send(Message::text(text)).await {
                    log::debug!("插件连接写出失败: {err}");
                    break;
                }
            }

            // 上行：读取插件事件。
            received = socket.recv() => {
                let Some(frame) = received else { break };
                let frame = match frame {
                    Ok(frame) => frame,
                    Err(err) => {
                        log::debug!("插件连接读取失败: {err}");
                        break;
                    }
                };

                if frame.is_close() {
                    break;
                }

                let Ok(text) = frame.as_str() else {
                    log::debug!("忽略来自插件的非文本帧: {} 字节", frame.as_bytes().len());
                    continue;
                };

                match serde_json::from_str(text) {
                    Ok(event) => state.handle_plugin_event(event),
                    Err(err) => log::warn!("无法解析插件事件: {err}; 原始内容: {text}"),
                }
            }
        }
    }

    state.unregister_plugin(connection_id);
    log::info!("插件已断开: {connection_id}");

    // 插件下线后刷新在线状态。
    state.broadcast(state.ready_message().await);
}
