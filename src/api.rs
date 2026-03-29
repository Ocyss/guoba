use anyhow::{Context, Result, anyhow};
use salvo::http::StatusCode;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::OnceLock;

use crate::core::SharedCore;
use crate::{core_control, core_manager, lsp_bridge, subscription, yaml_overlay};

static CORE: OnceLock<SharedCore> = OnceLock::new();

#[derive(Debug, Serialize)]
struct ApiOk<T: Serialize> {
    ok: bool,
    data: T,
}

#[derive(Debug, Serialize)]
struct ApiErr {
    ok: bool,
    error: String,
}

#[derive(Debug, Clone, Deserialize)]
struct SaveContentRequest {
    content: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ApplyRequest {
    action: Option<ApplyAction>,
    override_content: Option<String>,
    source_content: Option<String>,
    source_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct PreviewRequest {
    override_content: Option<String>,
    source_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ApplyAction {
    None,
    Reload,
    Restart,
}

impl Default for ApplyAction {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, Serialize)]
struct ConfigDocument {
    path: String,
    content: String,
}

#[derive(Debug, Clone, Serialize)]
struct RenderResult {
    source_path: String,
    override_path: String,
    rendered_path: String,
    content: String,
}

#[derive(Debug, Clone, Serialize)]
struct ApplyResult {
    rendered: RenderResult,
    action: ApplyAction,
    core_status: core_control::CoreStatus,
}

#[derive(Debug, Clone, Serialize)]
struct PreviewResult {
    content: String,
}

#[derive(Debug, Clone, Serialize)]
struct BootstrapData {
    status: core_control::CoreStatus,
    override_doc: ConfigDocument,
    subscriptions: subscription::SubscriptionView,
    core_versions: core_manager::CoreVersionList,
    runtime_yaml: String,
    source_yaml: String,
    preview_yaml: String,
}

pub fn init(core: SharedCore) {
    let _ = CORE.set(core);
}

pub fn router() -> Router {
    Router::with_path("api")
        .push(Router::with_path("ping").get(ping))
        .push(Router::with_path("editor").push(Router::with_path("ws").goal(lsp_bridge::editor_ws)))
        .push(Router::with_path("bootstrap").get(get_bootstrap))
        .push(
            Router::with_path("config")
                .push(
                    Router::with_path("override")
                        .get(get_override_config)
                        .post(save_override_config),
                )
                .push(Router::with_path("preview").post(preview_config))
                .push(Router::with_path("apply").post(apply_config)),
        )
        .push(
            Router::with_path("core")
                .push(Router::with_path("status").get(get_core_status))
                .push(Router::with_path("reload").post(reload_core))
                .push(Router::with_path("restart").post(restart_core))
                .push(Router::with_path("versions").get(get_core_versions))
                .push(Router::with_path("install").post(install_core))
                .push(Router::with_path("sync").post(sync_core_from_github))
                .push(Router::with_path("select").post(select_core_version))
                .push(Router::with_path("auto-update").post(set_core_auto_update))
                .push(Router::with_path("keep-versions").post(set_core_keep_versions)),
        )
        .push(
            Router::with_path("subscriptions")
                .get(get_subscriptions)
                .push(Router::with_path("items").post(save_subscription_items))
                .push(Router::with_path("local").post(save_local_subscription))
                .push(Router::with_path("content").post(get_subscription_content))
                .push(Router::with_path("pull").post(pull_subscriptions)),
        )
}

#[handler]
async fn ping(res: &mut Response) {
    respond_ok(res, serde_json::json!({"pong": true}));
}

#[handler]
async fn get_bootstrap(res: &mut Response) {
    let core = match core_ref() {
        Ok(core) => core,
        Err(err) => {
            respond_err(res, StatusCode::INTERNAL_SERVER_ERROR, err);
            return;
        }
    };

    let status = match core_control::get_status(core) {
        Ok(data) => data,
        Err(err) => {
            respond_err(res, StatusCode::INTERNAL_SERVER_ERROR, err);
            return;
        }
    };

    let override_doc = match read_config_doc(&core.read().unwrap().path.override_config) {
        Ok(data) => data,
        Err(err) => {
            respond_err(res, StatusCode::INTERNAL_SERVER_ERROR, err);
            return;
        }
    };

    let subscriptions = match subscription::get_subscription_view(core) {
        Ok(data) => data,
        Err(err) => {
            respond_err(res, StatusCode::INTERNAL_SERVER_ERROR, err);
            return;
        }
    };

    let core_versions = match core_manager::list_versions(core) {
        Ok(data) => data,
        Err(err) => {
            respond_err(res, StatusCode::INTERNAL_SERVER_ERROR, err);
            return;
        }
    };

    let mut source_yaml = subscriptions.local_content.clone();
    if subscriptions.default_id != subscriptions.local_id {
        if let Ok(content) = subscription::read_subscription_content(
            core,
            subscription::SubscriptionContentRequest {
                id: Some(subscriptions.default_id.clone()),
                name: None,
            },
        ) {
            source_yaml = content.content;
        }
    }

    let preview_yaml = yaml_overlay::render_yaml(&source_yaml, &override_doc.content)
        .unwrap_or_else(|_| override_doc.content.clone());

    let runtime_yaml =
        std::fs::read_to_string(&status.runtime_config).unwrap_or_else(|_| preview_yaml.clone());

    respond_ok(
        res,
        BootstrapData {
            status,
            override_doc,
            subscriptions,
            core_versions,
            runtime_yaml,
            source_yaml,
            preview_yaml,
        },
    );
}

#[handler]
async fn preview_config(req: &mut Request, res: &mut Response) {
    let body = match req.parse_json::<PreviewRequest>().await {
        Ok(body) => body,
        Err(err) => {
            respond_err(
                res,
                StatusCode::BAD_REQUEST,
                anyhow!("请求体解析失败: {err}"),
            );
            return;
        }
    };

    let preview = match with_core(|core| {
        core_control::render_preview_yaml(
            core,
            body.source_content.as_deref(),
            body.override_content.as_deref(),
        )
    }) {
        Ok(content) => PreviewResult { content },
        Err(err) => {
            respond_err(res, StatusCode::BAD_REQUEST, err);
            return;
        }
    };

    respond_ok(res, preview);
}

#[handler]
async fn get_override_config(res: &mut Response) {
    match with_core(|core| read_config_doc(&core.read().unwrap().path.override_config)) {
        Ok(doc) => respond_ok(res, doc),
        Err(err) => respond_err(res, StatusCode::INTERNAL_SERVER_ERROR, err),
    }
}

#[handler]
async fn save_override_config(req: &mut Request, res: &mut Response) {
    let body = match req.parse_json::<SaveContentRequest>().await {
        Ok(body) => body,
        Err(err) => {
            respond_err(
                res,
                StatusCode::BAD_REQUEST,
                anyhow!("请求体解析失败: {err}"),
            );
            return;
        }
    };

    match with_core(|core| {
        let path = core.read().unwrap().path.override_config.clone();
        std::fs::write(&path, &body.content)
            .with_context(|| format!("写入覆写配置失败: {}", path.display()))?;
        read_config_doc(&path)
    }) {
        Ok(doc) => respond_ok(res, doc),
        Err(err) => respond_err(res, StatusCode::BAD_REQUEST, err),
    }
}

#[handler]
async fn apply_config(req: &mut Request, res: &mut Response) {
    let body = match req.parse_json::<ApplyRequest>().await {
        Ok(body) => body,
        Err(err) => {
            respond_err(
                res,
                StatusCode::BAD_REQUEST,
                anyhow!("请求体解析失败: {err}"),
            );
            return;
        }
    };

    let action = body.action.unwrap_or_default();

    let rendered = match with_core(|core| {
        if let Some(override_content) = body.override_content.as_deref() {
            let path = core.read().unwrap().path.override_config.clone();
            std::fs::write(&path, override_content)
                .with_context(|| format!("写入覆写配置失败: {}", path.display()))?;
        }
        let render_info = core_control::render_runtime_config(
            core,
            body.source_content.as_deref(),
            body.source_id.as_deref(),
        )?;
        let content = std::fs::read_to_string(&render_info.rendered_path)
            .with_context(|| format!("读取渲染配置失败: {}", render_info.rendered_path))?;
        Ok(RenderResult {
            source_path: render_info.source_path,
            override_path: render_info.override_path,
            rendered_path: render_info.rendered_path,
            content,
        })
    }) {
        Ok(rendered) => rendered,
        Err(err) => {
            respond_err(res, StatusCode::BAD_REQUEST, err);
            return;
        }
    };

    let core_ref = match core_ref() {
        Ok(core) => core,
        Err(err) => {
            respond_err(res, StatusCode::INTERNAL_SERVER_ERROR, err);
            return;
        }
    };

    let action_result = match action {
        ApplyAction::None => Ok(()),
        ApplyAction::Reload => core_control::hot_reload(core_ref).await,
        ApplyAction::Restart => core_control::restart(core_ref).map(|_| ()),
    };
    if let Err(err) = action_result {
        respond_err(res, StatusCode::BAD_GATEWAY, err);
        return;
    }

    match core_control::get_status(core_ref) {
        Ok(core_status) => respond_ok(
            res,
            ApplyResult {
                rendered,
                action,
                core_status,
            },
        ),
        Err(err) => respond_err(res, StatusCode::INTERNAL_SERVER_ERROR, err),
    }
}

#[handler]
async fn get_core_status(res: &mut Response) {
    match core_ref().and_then(core_control::get_status) {
        Ok(status) => respond_ok(res, status),
        Err(err) => respond_err(res, StatusCode::INTERNAL_SERVER_ERROR, err),
    }
}

#[handler]
async fn reload_core(res: &mut Response) {
    let core = match core_ref() {
        Ok(core) => core,
        Err(err) => {
            respond_err(res, StatusCode::INTERNAL_SERVER_ERROR, err);
            return;
        }
    };

    match core_control::hot_reload(core).await {
        Ok(_) => match core_control::get_status(core) {
            Ok(status) => respond_ok(res, status),
            Err(err) => respond_err(res, StatusCode::INTERNAL_SERVER_ERROR, err),
        },
        Err(err) => respond_err(res, StatusCode::BAD_GATEWAY, err),
    }
}

#[handler]
async fn restart_core(res: &mut Response) {
    match core_ref().and_then(core_control::restart) {
        Ok(status) => respond_ok(res, status),
        Err(err) => respond_err(res, StatusCode::BAD_GATEWAY, err),
    }
}

#[handler]
async fn get_core_versions(res: &mut Response) {
    match core_ref().and_then(core_manager::list_versions) {
        Ok(data) => respond_ok(res, data),
        Err(err) => respond_err(res, StatusCode::INTERNAL_SERVER_ERROR, err),
    }
}

#[handler]
async fn install_core(req: &mut Request, res: &mut Response) {
    let body = match req.parse_json::<core_manager::InstallCoreRequest>().await {
        Ok(body) => body,
        Err(err) => {
            respond_err(
                res,
                StatusCode::BAD_REQUEST,
                anyhow!("请求体解析失败: {err}"),
            );
            return;
        }
    };

    let core = match core_ref() {
        Ok(core) => core,
        Err(err) => {
            respond_err(res, StatusCode::INTERNAL_SERVER_ERROR, err);
            return;
        }
    };

    match core_manager::install_core(core, body).await {
        Ok(data) => respond_ok(res, data),
        Err(err) => respond_err(res, StatusCode::BAD_REQUEST, err),
    }
}

#[handler]
async fn sync_core_from_github(res: &mut Response) {
    let core = match core_ref() {
        Ok(core) => core,
        Err(err) => {
            respond_err(res, StatusCode::INTERNAL_SERVER_ERROR, err);
            return;
        }
    };

    match core_manager::sync_latest_from_github(core).await {
        Ok(data) => respond_ok(res, data),
        Err(err) => respond_err(res, StatusCode::BAD_GATEWAY, err),
    }
}

#[handler]
async fn select_core_version(req: &mut Request, res: &mut Response) {
    let body = match req.parse_json::<core_manager::SelectVersionRequest>().await {
        Ok(body) => body,
        Err(err) => {
            respond_err(
                res,
                StatusCode::BAD_REQUEST,
                anyhow!("请求体解析失败: {err}"),
            );
            return;
        }
    };

    match core_ref().and_then(|core| core_manager::select_version(core, body)) {
        Ok(data) => respond_ok(res, data),
        Err(err) => respond_err(res, StatusCode::BAD_REQUEST, err),
    }
}

#[handler]
async fn set_core_auto_update(req: &mut Request, res: &mut Response) {
    let body = match req.parse_json::<core_manager::AutoUpdateRequest>().await {
        Ok(body) => body,
        Err(err) => {
            respond_err(
                res,
                StatusCode::BAD_REQUEST,
                anyhow!("请求体解析失败: {err}"),
            );
            return;
        }
    };

    match core_ref().and_then(|core| core_manager::set_auto_update(core, body)) {
        Ok(data) => respond_ok(res, data),
        Err(err) => respond_err(res, StatusCode::BAD_REQUEST, err),
    }
}

#[handler]
async fn set_core_keep_versions(req: &mut Request, res: &mut Response) {
    let body = match req.parse_json::<core_manager::KeepVersionsRequest>().await {
        Ok(body) => body,
        Err(err) => {
            respond_err(
                res,
                StatusCode::BAD_REQUEST,
                anyhow!("请求体解析失败: {err}"),
            );
            return;
        }
    };

    match core_ref().and_then(|core| core_manager::set_keep_versions(core, body)) {
        Ok(data) => respond_ok(res, data),
        Err(err) => respond_err(res, StatusCode::BAD_REQUEST, err),
    }
}

#[handler]
async fn get_subscriptions(res: &mut Response) {
    match core_ref().and_then(subscription::get_subscription_view) {
        Ok(data) => respond_ok(res, data),
        Err(err) => respond_err(res, StatusCode::INTERNAL_SERVER_ERROR, err),
    }
}

#[handler]
async fn save_subscription_items(req: &mut Request, res: &mut Response) {
    let body = match req.parse_json::<subscription::SaveItemsRequest>().await {
        Ok(body) => body,
        Err(err) => {
            respond_err(
                res,
                StatusCode::BAD_REQUEST,
                anyhow!("请求体解析失败: {err}"),
            );
            return;
        }
    };

    match core_ref().and_then(|core| subscription::save_subscription_items(core, body)) {
        Ok(data) => respond_ok(res, data),
        Err(err) => respond_err(res, StatusCode::BAD_REQUEST, err),
    }
}

#[handler]
async fn save_local_subscription(req: &mut Request, res: &mut Response) {
    let body = match req.parse_json::<subscription::SaveLocalRequest>().await {
        Ok(body) => body,
        Err(err) => {
            respond_err(
                res,
                StatusCode::BAD_REQUEST,
                anyhow!("请求体解析失败: {err}"),
            );
            return;
        }
    };

    match core_ref().and_then(|core| subscription::save_local_subscription(core, body)) {
        Ok(data) => respond_ok(res, data),
        Err(err) => respond_err(res, StatusCode::BAD_REQUEST, err),
    }
}

#[handler]
async fn get_subscription_content(req: &mut Request, res: &mut Response) {
    let body = match req
        .parse_json::<subscription::SubscriptionContentRequest>()
        .await
    {
        Ok(body) => body,
        Err(err) => {
            respond_err(
                res,
                StatusCode::BAD_REQUEST,
                anyhow!("请求体解析失败: {err}"),
            );
            return;
        }
    };

    match core_ref().and_then(|core| subscription::read_subscription_content(core, body)) {
        Ok(data) => respond_ok(res, data),
        Err(err) => respond_err(res, StatusCode::BAD_REQUEST, err),
    }
}

#[handler]
async fn pull_subscriptions(res: &mut Response) {
    let core = match core_ref() {
        Ok(core) => core,
        Err(err) => {
            respond_err(res, StatusCode::INTERNAL_SERVER_ERROR, err);
            return;
        }
    };

    match subscription::pull_all(core).await {
        Ok(report) => respond_ok(res, report),
        Err(err) => respond_err(res, StatusCode::BAD_GATEWAY, err),
    }
}

fn read_config_doc(path: &Path) -> Result<ConfigDocument> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, "")?;
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("读取配置文件失败: {}", path.display()))?;

    Ok(ConfigDocument {
        path: path.display().to_string(),
        content,
    })
}

fn core_ref() -> Result<&'static SharedCore> {
    CORE.get().ok_or_else(|| anyhow!("核心状态未初始化"))
}

fn with_core<T>(f: impl FnOnce(&SharedCore) -> Result<T>) -> Result<T> {
    let core = core_ref()?;
    f(core)
}

fn respond_ok<T: Serialize + Send + Sync + 'static>(res: &mut Response, data: T) {
    res.status_code(StatusCode::OK);
    res.render(Json(ApiOk { ok: true, data }));
}

fn respond_err(res: &mut Response, code: StatusCode, err: anyhow::Error) {
    log::error!("api error: {err:#}");
    res.status_code(code);
    res.render(Json(ApiErr {
        ok: false,
        error: err.to_string(),
    }));
}
