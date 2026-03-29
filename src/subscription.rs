use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::{SharedCore, SubscriptionItem};

pub const LOCAL_SUBSCRIPTION_ID: &str = "local";

#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionPullItem {
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub success: bool,
    pub file: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionPullReport {
    pub total: usize,
    pub updated: usize,
    pub failed: usize,
    pub items: Vec<SubscriptionPullItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionView {
    pub interval_minutes: u64,
    pub keep_versions: usize,
    pub items: Vec<SubscriptionItem>,
    pub local_id: String,
    pub default_id: String,
    pub local_file: String,
    pub local_content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SaveLocalRequest {
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SaveItemsRequest {
    pub interval_minutes: Option<u64>,
    pub keep_versions: Option<usize>,
    pub items: Vec<SubscriptionItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubscriptionContentRequest {
    pub id: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionContent {
    pub id: String,
    pub name: String,
    pub path: String,
    pub content: String,
}

pub fn get_subscription_view(core: &SharedCore) -> Result<SubscriptionView> {
    let (local_file, local_content, interval_minutes, keep_versions, items) = {
        let mut guard = core.write().unwrap();
        let local_file = guard.path.subscription_local_file.clone();
        ensure_local_file(&local_file)?;
        let normalized_items = normalize_subscription_items(&guard.config.subscription_items);
        if guard.config.subscription_items != normalized_items {
            guard.config.subscription_items = normalized_items.clone();
            guard.save_config();
        }
        (
            local_file.clone(),
            fs::read_to_string(&local_file).unwrap_or_default(),
            guard.config.interval_minutes,
            guard.config.subscription_keep_versions.max(1),
            normalized_items,
        )
    };

    Ok(SubscriptionView {
        interval_minutes,
        keep_versions,
        items,
        local_id: LOCAL_SUBSCRIPTION_ID.to_string(),
        default_id: LOCAL_SUBSCRIPTION_ID.to_string(),
        local_file: local_file.display().to_string(),
        local_content,
    })
}

pub fn save_local_subscription(
    core: &SharedCore,
    req: SaveLocalRequest,
) -> Result<SubscriptionView> {
    let local_file = {
        let guard = core.read().unwrap();
        guard.path.subscription_local_file.clone()
    };

    ensure_local_file(&local_file)?;
    fs::write(&local_file, req.content)
        .with_context(|| format!("写入本地订阅失败: {}", local_file.display()))?;

    get_subscription_view(core)
}

pub fn save_subscription_items(
    core: &SharedCore,
    req: SaveItemsRequest,
) -> Result<SubscriptionView> {
    {
        let mut guard = core.write().unwrap();
        guard.config.subscription_items = normalize_subscription_items(&req.items);
        if let Some(interval_minutes) = req.interval_minutes {
            guard.config.interval_minutes = interval_minutes;
        }
        if let Some(keep_versions) = req.keep_versions {
            guard.config.subscription_keep_versions = keep_versions.max(1);
        }
        guard.save_config();
    }

    get_subscription_view(core)
}

pub fn read_subscription_content(
    core: &SharedCore,
    req: SubscriptionContentRequest,
) -> Result<SubscriptionContent> {
    let requested_id = req.id.unwrap_or_default();
    let requested_name = req.name.unwrap_or_default();
    let key = if requested_id.trim().is_empty() {
        requested_name.trim().to_string()
    } else {
        requested_id.trim().to_string()
    };
    if key.is_empty() {
        return Err(anyhow!("订阅标识不能为空"));
    }

    let (resolved_id, resolved_name, latest_file) = {
        let guard = core.read().unwrap();
        if key == LOCAL_SUBSCRIPTION_ID {
            (
                LOCAL_SUBSCRIPTION_ID.to_string(),
                "本地订阅".to_string(),
                guard.path.subscription_local_file.clone(),
            )
        } else {
            let item = guard
                .config
                .subscription_items
                .iter()
                .find(|x| x.id == key || x.name == key)
                .ok_or_else(|| anyhow!("订阅不存在: {}", key))?;
            (
                item.id.clone(),
                item.name.clone(),
                guard
                    .path
                    .subscription_dir
                    .join(item_folder_name(item))
                    .join("latest.yaml"),
            )
        }
    };

    let content = fs::read_to_string(&latest_file)
        .with_context(|| format!("读取订阅内容失败: {}", latest_file.display()))?;

    Ok(SubscriptionContent {
        id: resolved_id,
        name: resolved_name,
        path: latest_file.display().to_string(),
        content,
    })
}

pub async fn pull_all(core: &SharedCore) -> Result<SubscriptionPullReport> {
    let (subscription_dir, items, keep_versions) = {
        let guard = core.read().unwrap();
        (
            guard.path.subscription_dir.clone(),
            guard.config.subscription_items.clone(),
            guard.config.subscription_keep_versions.max(1),
        )
    };

    if !subscription_dir.exists() {
        std::fs::create_dir_all(&subscription_dir)
            .with_context(|| format!("创建订阅目录失败: {}", subscription_dir.display()))?;
    }

    let mut result_items = Vec::with_capacity(items.len());
    let mut updated = 0usize;
    let mut failed = 0usize;

    for item in &items {
        if !item.enabled {
            result_items.push(SubscriptionPullItem {
                name: item.name.clone(),
                url: item.url.clone(),
                enabled: false,
                success: true,
                file: None,
                error: None,
            });
            continue;
        }

        let folder = subscription_dir.join(item_folder_name(item));
        let latest_file = folder.join("latest.yaml");

        match pull_one(&item.url).await {
            Ok(content) => {
                let changed = write_remote_subscription(&latest_file, &content, keep_versions)?;
                if changed {
                    updated += 1;
                }
                result_items.push(SubscriptionPullItem {
                    name: item.name.clone(),
                    url: item.url.clone(),
                    enabled: true,
                    success: true,
                    file: Some(latest_file.display().to_string()),
                    error: None,
                });
            }
            Err(err) => {
                failed += 1;
                result_items.push(SubscriptionPullItem {
                    name: item.name.clone(),
                    url: item.url.clone(),
                    enabled: true,
                    success: false,
                    file: None,
                    error: Some(err.to_string()),
                });
            }
        }
    }

    Ok(SubscriptionPullReport {
        total: items.len(),
        updated,
        failed,
        items: result_items,
    })
}

fn ensure_local_file(path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建订阅目录失败: {}", parent.display()))?;
    }
    fs::write(
        path,
        "# 本地订阅配置\nmixed-port: 7890\nmode: rule\nlog-level: info\n",
    )
    .with_context(|| format!("创建本地订阅文件失败: {}", path.display()))?;
    Ok(())
}

fn write_remote_subscription(
    latest_file: &PathBuf,
    content: &str,
    keep_versions: usize,
) -> Result<bool> {
    if let Some(parent) = latest_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建订阅目录失败: {}", parent.display()))?;
    }

    let old_content = fs::read_to_string(latest_file).unwrap_or_default();
    if old_content == content {
        return Ok(false);
    }

    if !old_content.is_empty() {
        let history_dir = latest_file
            .parent()
            .ok_or_else(|| anyhow!("订阅目录异常"))?
            .join("history");
        fs::create_dir_all(&history_dir)
            .with_context(|| format!("创建订阅历史目录失败: {}", history_dir.display()))?;
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let history_file = history_dir.join(format!("{ts}.yaml"));
        fs::write(&history_file, old_content)
            .with_context(|| format!("写入订阅历史失败: {}", history_file.display()))?;
        prune_history_files(&history_dir, keep_versions.max(1))?;
    }

    fs::write(latest_file, content)
        .with_context(|| format!("写入最新订阅失败: {}", latest_file.display()))?;
    Ok(true)
}

fn prune_history_files(history_dir: &Path, keep_versions: usize) -> Result<()> {
    let mut files: Vec<(PathBuf, SystemTime)> = fs::read_dir(history_dir)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((path, modified))
        })
        .collect();

    if files.len() <= keep_versions {
        return Ok(());
    }

    files.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
    for (idx, (path, _)) in files.into_iter().enumerate() {
        if idx < keep_versions {
            continue;
        }
        let _ = fs::remove_file(path);
    }

    Ok(())
}

async fn pull_one(url: &str) -> Result<String> {
    let response = reqwest::get(url)
        .await
        .with_context(|| format!("请求订阅失败: {url}"))?;
    let response = response
        .error_for_status()
        .with_context(|| format!("订阅返回错误状态: {url}"))?;

    let content = response
        .text()
        .await
        .with_context(|| format!("读取订阅内容失败: {url}"))?;

    Ok(content)
}

fn sanitize_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            result.push(ch);
        } else {
            result.push('_');
        }
    }

    if result.is_empty() {
        "subscription".to_string()
    } else {
        result
    }
}

fn normalize_subscription_items(items: &[SubscriptionItem]) -> Vec<SubscriptionItem> {
    let mut used = HashSet::new();
    let mut next_counter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let mut id = sanitize_name(item.id.trim());
        if id.is_empty() || id == "subscription" || id == LOCAL_SUBSCRIPTION_ID {
            id = format!("sub-{next_counter}");
            next_counter += 1;
        }
        while used.contains(&id) {
            id = format!("sub-{next_counter}");
            next_counter += 1;
        }
        used.insert(id.clone());

        out.push(SubscriptionItem {
            id,
            name: item.name.trim().to_string(),
            url: item.url.trim().to_string(),
            enabled: item.enabled,
        });
    }

    out
}

fn item_folder_name(item: &SubscriptionItem) -> String {
    let id = sanitize_name(item.id.trim());
    if id.is_empty() || id == "subscription" {
        sanitize_name(item.name.trim())
    } else {
        id
    }
}
