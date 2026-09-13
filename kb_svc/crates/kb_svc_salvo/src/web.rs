//! 面向浏览器的 HTTP / WebSocket 路由。
//!
//! | 路由 | 说明 |
//! | :--- | :--- |
//! | `GET /` | 聊天界面（内嵌 HTML） |
//! | `GET /app.css`、`GET /app.js` | 前端资源 |
//! | `GET /api/settings` | 读取设置（API key 遮蔽） |
//! | `POST /api/settings/services` | 新增或覆盖一个 LLM 服务 |
//! | `DELETE /api/settings/services/{id}` | 删除一个服务 |
//! | `POST /api/settings/active` | 切换当前生效的服务 |
//! | `GET /ws/chat` | 聊天事件通道 |
//!
//! 插件侧的路由见 [`crate::plugin`]，两者共用同一份 [`crate::server::router`]，
//! 因此 TCP 与 Unix socket 两个监听器都能访问。

use std::sync::Arc;

use salvo::{Depot, Request, Response, Router, http::StatusCode, prelude::*, writing::Text};

use crate::{
    assets::AssetSettings,
    hub::AppState,
    settings::{LlmServiceConfig, MASKED_KEY},
};

/// `POST /api/settings/services` 的请求体。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ServiceUpdate {
    /// 服务标识（用户可见的名字）。
    pub id: String,

    /// provider 标识。
    pub provider: String,

    /// 模型名。
    pub model: String,

    /// API base URL，可为空。
    #[serde(default)]
    pub base_url: String,

    /// API key。
    ///
    /// 若传回的是遮蔽值（`••••••••`），服务端会保留原有 key 而不是写入遮蔽串；
    /// 这样界面上「只改模型名」不会误清空 key。
    #[serde(default)]
    pub api_key: String,
}

/// `POST /api/settings/active` 的请求体。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ActiveServiceUpdate {
    /// 目标服务标识。
    pub service_id: String,
}

/// 构造浏览器侧的路由。
pub fn router() -> Router {
    Router::new()
        .get(index)
        .push(Router::with_path("app.css").get(app_css))
        .push(Router::with_path("app.js").get(app_js))
        .push(
            Router::with_path("api/settings")
                .get(get_settings)
                .push(Router::with_path("services").post(upsert_service))
                .push(Router::with_path("services/{id}").delete(delete_service))
                .push(Router::with_path("active").post(set_active_service)),
        )
        .push(Router::with_path("ws/chat").goal(chat_ws))
}

/// 从 `Depot` 取出共享状态。
fn state(depot: &Depot) -> Result<Arc<AppState>, StatusError> {
    depot
        .get_typed::<Arc<AppState>>()
        .cloned()
        .map_err(|_| StatusError::internal_server_error().brief("AppState 未注入"))
}

/// 从 `Depot` 取出资源设置。
fn asset_settings(depot: &Depot) -> AssetSettings {
    depot
        .get_typed::<AssetSettings>()
        .cloned()
        .unwrap_or_default()
}

/// 把任意可序列化的值渲染成 JSON 响应。
fn json_response<T: serde::Serialize>(
    res: &mut Response,
    status: StatusCode,
    value: &T,
) -> Result<(), StatusError> {
    let body = serde_json::to_string(value).map_err(|err| {
        StatusError::internal_server_error().brief(format!("JSON 序列化失败: {err}"))
    })?;

    res.status_code(status);
    res.render(Text::Json(body));
    Ok(())
}

/// 渲染一条错误 JSON。
fn error_response(res: &mut Response, status: StatusCode, message: impl Into<String>) {
    let body = serde_json::json!({ "error": message.into() }).to_string();
    res.status_code(status);
    res.render(Text::Json(body));
}

/// `GET /`：聊天界面。
#[handler]
async fn index(res: &mut Response, depot: &mut Depot) {
    let html = asset_settings(depot)
        .resolve(&crate::assets::INDEX_ASSET)
        .await;
    res.render(Text::Html(html));
}

/// `GET /app.css`：样式表。
#[handler]
async fn app_css(res: &mut Response, depot: &mut Depot) {
    let css = asset_settings(depot)
        .resolve(&crate::assets::ASSETS[0])
        .await;
    res.render(Text::Css(css));
}

/// `GET /app.js`：前端脚本。
#[handler]
async fn app_js(res: &mut Response, depot: &mut Depot) {
    let js = asset_settings(depot)
        .resolve(&crate::assets::ASSETS[1])
        .await;
    res.render(Text::Js(js));
}

/// `GET /api/settings`：返回服务列表与当前生效服务。
///
/// API key 会被遮蔽，只保留 `has_api_key` 供界面判断。
#[handler]
async fn get_settings(res: &mut Response, depot: &mut Depot) -> Result<(), StatusError> {
    let state = state(depot)?;

    let settings = state.store().load().await.map_err(|err| {
        StatusError::internal_server_error().brief(format!("读取设置失败: {err}"))
    })?;

    let services: Vec<serde_json::Value> = settings
        .services
        .iter()
        .map(|(id, service)| {
            let masked = service.masked();
            serde_json::json!({
                "id": id,
                "provider": masked.provider,
                "model": masked.model,
                "base_url": masked.base_url,
                "api_key": masked.api_key,
                "has_api_key": service.has_api_key(),
            })
        })
        .collect();

    json_response(
        res,
        StatusCode::OK,
        &serde_json::json!({
            "active_service": state.active_service(),
            "services": services,
            "config_path": state.store().path().map(|path| path.display().to_string()),
        }),
    )
}

/// `POST /api/settings/services`：新增或覆盖一个服务。
#[handler]
async fn upsert_service(
    req: &mut Request,
    res: &mut Response,
    depot: &mut Depot,
) -> Result<(), StatusError> {
    let state = state(depot)?;

    let update: ServiceUpdate = match req.parse_json().await {
        Ok(update) => update,
        Err(err) => {
            error_response(
                res,
                StatusCode::BAD_REQUEST,
                format!("请求体解析失败: {err}"),
            );
            return Ok(());
        }
    };

    if update.id.trim().is_empty() {
        error_response(res, StatusCode::BAD_REQUEST, "服务标识不能为空");
        return Ok(());
    }

    if update.provider.trim().is_empty() {
        error_response(res, StatusCode::BAD_REQUEST, "provider 不能为空");
        return Ok(());
    }

    // 界面上回填的遮蔽值不代表用户想改 key，应当保留原有值。
    let api_key = if update.api_key.trim() == MASKED_KEY {
        state
            .store()
            .load()
            .await
            .ok()
            .and_then(|settings| settings.services.get(&update.id).map(|s| s.api_key.clone()))
            .unwrap_or_default()
    } else {
        update.api_key.trim().to_string()
    };

    let service = LlmServiceConfig::new(
        update.provider.trim(),
        update.model.trim(),
        update.base_url.trim(),
        api_key,
    );

    if let Err(err) = state.store().upsert_service(&update.id, &service).await {
        error_response(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("写入设置失败: {err}"),
        );
        return Ok(());
    }

    // 新增的服务如果还没有生效服务，就顺手设为生效服务。
    if state.active_service().is_none() && service.has_api_key() {
        let _ = state.use_service(&update.id).await;
    }

    json_response(
        res,
        StatusCode::OK,
        &serde_json::json!({ "ok": true, "id": update.id }),
    )
}

/// `DELETE /api/settings/services/{id}`：删除一个服务。
#[handler]
async fn delete_service(
    req: &mut Request,
    res: &mut Response,
    depot: &mut Depot,
) -> Result<(), StatusError> {
    let state = state(depot)?;

    let Some(id) = req.param::<String>("id") else {
        error_response(res, StatusCode::BAD_REQUEST, "缺少服务标识");
        return Ok(());
    };

    if let Err(err) = state.store().remove_service(&id).await {
        error_response(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("删除设置失败: {err}"),
        );
        return Ok(());
    }

    // 被删掉的正好是生效服务时，换一个可用的；没有就置空。
    if state.active_service().as_deref() == Some(id.as_str()) {
        let next = state.store().load().await.ok().and_then(|settings| {
            settings
                .services
                .iter()
                .find(|(_, service)| service.has_api_key())
                .map(|(id, _)| id.clone())
        });

        match next {
            Some(next) => {
                let _ = state.use_service(&next).await;
            }
            None => state.clear_active_service(),
        }
    }

    json_response(res, StatusCode::OK, &serde_json::json!({ "ok": true }))
}

/// `POST /api/settings/active`：切换当前生效的服务。
#[handler]
async fn set_active_service(
    req: &mut Request,
    res: &mut Response,
    depot: &mut Depot,
) -> Result<(), StatusError> {
    let state = state(depot)?;

    let update: ActiveServiceUpdate = match req.parse_json().await {
        Ok(update) => update,
        Err(err) => {
            error_response(
                res,
                StatusCode::BAD_REQUEST,
                format!("请求体解析失败: {err}"),
            );
            return Ok(());
        }
    };

    if let Err(err) = state.use_service(&update.service_id).await {
        error_response(res, StatusCode::BAD_REQUEST, err.message);
        return Ok(());
    }

    json_response(res, StatusCode::OK, &serde_json::json!({ "ok": true }))
}

/// `GET /ws/chat`：浏览器事件通道。
#[handler]
async fn chat_ws(req: &mut Request, res: &mut Response, depot: &mut Depot) {
    let Ok(state) = state(depot) else {
        res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
        return;
    };

    // 必须在升级之前订阅：否则「订阅建立」与「升级完成」之间的广播会丢失。
    let events = state.subscribe();

    let upgrade = salvo::websocket::WebSocketUpgrade::new();
    let result = upgrade
        .upgrade(req, res, move |socket| async move {
            crate::web_ws::serve_chat(socket, state, events).await;
        })
        .await;

    if let Err(err) = result {
        log::warn!("聊天 WebSocket 升级失败: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{hub::AppState, settings::SettingsStore};
    use salvo::test::{ResponseExt, TestClient};

    /// 构造一个带内存设置的测试服务。
    async fn test_service(with_override_dir: Option<&str>) -> salvo::Service {
        let state = std::sync::Arc::new(AppState::new(SettingsStore::memory()).await);
        let assets = match with_override_dir {
            Some(dir) => AssetSettings::with_override_dir(dir),
            None => AssetSettings::default(),
        };

        salvo::Service::new(router())
            .hoop(salvo::affix_state::inject(state))
            .hoop(salvo::affix_state::inject(assets))
    }

    /// 测试首页返回内嵌 HTML。
    ///
    /// - 手段：用 Salvo 的测试客户端请求 `GET /`。
    /// - 判断：状态码 200，`content-type` 含 `text/html`，响应体包含
    ///   `kb_svc_salvo` 与聊天界面标记 `id="chat"`。
    #[tokio::test]
    async fn index_serves_embedded_html() {
        let service = test_service(None).await;

        let mut res = TestClient::get("http://127.0.0.1/").send(&service).await;

        assert_eq!(res.status_code, Some(StatusCode::OK));
        let content_type = res
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(
            content_type.contains("text/html"),
            "实际 content-type: {content_type}"
        );

        let body = res.take_string().await.expect("应当有响应体");
        assert!(body.contains("llm_kb"), "首页应当含产品名");
        assert!(body.contains("id=\"chat\""), "首页应当含聊天区容器");
    }

    /// 测试 `GET /api/settings` 在没有任何服务时返回空列表。
    ///
    /// - 手段：内存设置（空）下请求该接口。
    /// - 判断：状态码 200；JSON 中 `services` 为空数组、`active_service` 为 null。
    #[tokio::test]
    async fn get_settings_returns_empty_list() {
        let service = test_service(None).await;

        let mut res = TestClient::get("http://127.0.0.1/api/settings")
            .send(&service)
            .await;

        assert_eq!(res.status_code, Some(StatusCode::OK));
        let body = res.take_string().await.expect("应当有响应体");
        let value: serde_json::Value = serde_json::from_str(&body).expect("应当是 JSON");

        assert_eq!(value["services"], serde_json::json!([]));
        assert!(value["active_service"].is_null());
    }
}
