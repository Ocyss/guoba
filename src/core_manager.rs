use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::{SharedCore, core_binary_file_name};

const MIHOMO_GITHUB_LATEST_API: &str =
    "https://api.github.com/repos/MetaCubeX/mihomo/releases/latest";

#[derive(Debug, Clone, Serialize)]
pub struct CoreVersionInfo {
    pub version: String,
    pub file: String,
    pub downloaded_unix: u64,
    pub released_at: Option<String>,
    pub source_tag: Option<String>,
    pub is_current: bool,
    pub is_local: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CoreVersionList {
    pub current: String,
    pub auto_update: bool,
    pub keep_versions: usize,
    pub versions: Vec<CoreVersionInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstallCoreRequest {
    #[serde(default)]
    pub version: String,
    pub url: String,
    #[serde(default)]
    pub set_current: bool,
    #[serde(default)]
    pub source_tag: Option<String>,
    #[serde(default)]
    pub released_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SelectVersionRequest {
    pub version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AutoUpdateRequest {
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KeepVersionsRequest {
    pub keep_versions: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GithubSyncResult {
    pub tag: String,
    pub asset_name: String,
    pub asset_url: String,
    pub release_published_at: Option<String>,
    pub versions: CoreVersionList,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    published_at: Option<String>,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    download_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CoreVersionMeta {
    downloaded_unix: u64,
    #[serde(default)]
    released_at: Option<String>,
    #[serde(default)]
    source_tag: Option<String>,
}

pub fn list_versions(core: &SharedCore) -> Result<CoreVersionList> {
    let (versions_dir, local_binary, current_bin, current, auto_update, keep_versions) = {
        let guard = core.read().unwrap();
        (
            guard.path.mihomo_versions_dir.clone(),
            guard.path.mihomo_local_binary.clone(),
            guard.path.mihomo_current_binary.clone(),
            guard.config.mihomo_version.clone(),
            guard.config.core_auto_update,
            guard.config.core_keep_versions.max(1),
        )
    };

    fs::create_dir_all(&versions_dir)
        .with_context(|| format!("创建内核版本目录失败: {}", versions_dir.display()))?;

    let mut versions = Vec::new();
    for entry in fs::read_dir(&versions_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let file_name = match path.file_name().and_then(|x| x.to_str()) {
            Some(x) => x,
            None => continue,
        };

        let Some(version) = extract_version(file_name) else {
            continue;
        };

        versions.push(build_version_info(&current_bin, &path, version, false));
    }

    if local_binary.exists() {
        versions.push(build_version_info(
            &current_bin,
            &local_binary,
            "local".to_string(),
            true,
        ));
    }

    versions.sort_by_key(|x| std::cmp::Reverse(x.downloaded_unix));

    Ok(CoreVersionList {
        current,
        auto_update,
        keep_versions,
        versions,
    })
}

pub async fn install_core(core: &SharedCore, req: InstallCoreRequest) -> Result<CoreVersionList> {
    if req.url.trim().is_empty() {
        return Err(anyhow!("url 不能为空"));
    }

    let is_http = is_http_url(&req.url);
    if !is_http {
        let local_source = PathBuf::from(req.url.trim());
        return install_local_core(core, &local_source, req.set_current);
    }

    if req.version.trim().is_empty() {
        return Err(anyhow!("远程下载时 version 不能为空"));
    }

    let (versions_dir, keep_versions) = {
        let guard = core.read().unwrap();
        (
            guard.path.mihomo_versions_dir.clone(),
            guard.config.core_keep_versions.max(1),
        )
    };

    fs::create_dir_all(&versions_dir)
        .with_context(|| format!("创建内核版本目录失败: {}", versions_dir.display()))?;

    let version = req.version.trim().to_string();
    let target_file = versions_dir.join(core_binary_file_name(&version));
    let response = reqwest::get(&req.url)
        .await
        .with_context(|| format!("下载内核失败: {}", req.url))?
        .error_for_status()
        .with_context(|| format!("下载内核返回错误状态: {}", req.url))?;
    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("读取内核内容失败: {}", req.url))?;

    write_core_binary(&target_file, &bytes)
        .with_context(|| format!("安装内核失败: {}", target_file.display()))?;

    #[cfg(not(target_os = "windows"))]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&target_file)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&target_file, perms)?;
    }

    write_meta_for(
        &target_file,
        CoreVersionMeta {
            downloaded_unix: now_unix(),
            released_at: req.released_at.clone(),
            source_tag: req.source_tag.clone(),
        },
    )?;

    if req.set_current {
        select_version(
            core,
            SelectVersionRequest {
                version: version.clone(),
            },
        )?;
    }

    prune_versions(core, keep_versions)?;
    list_versions(core)
}

pub fn install_local_core(
    core: &SharedCore,
    source_path: &Path,
    set_current: bool,
) -> Result<CoreVersionList> {
    let (local_binary, keep_versions) = {
        let guard = core.read().unwrap();
        (
            guard.path.mihomo_local_binary.clone(),
            guard.config.core_keep_versions.max(1),
        )
    };

    if !source_path.exists() {
        return Err(anyhow!("本地核心文件不存在: {}", source_path.display()));
    }

    if source_path != local_binary {
        if let Some(parent) = local_binary.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("创建本地核心目录失败: {}", parent.display()))?;
        }
        fs::copy(source_path, &local_binary).with_context(|| {
            format!(
                "复制本地核心失败: {} -> {}",
                source_path.display(),
                local_binary.display()
            )
        })?;
    }

    write_meta_for(
        &local_binary,
        CoreVersionMeta {
            downloaded_unix: now_unix(),
            released_at: None,
            source_tag: Some("local".to_string()),
        },
    )?;

    #[cfg(not(target_os = "windows"))]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&local_binary)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&local_binary, perms)?;
    }

    if set_current {
        select_version(
            core,
            SelectVersionRequest {
                version: "local".to_string(),
            },
        )?;
    }

    prune_versions(core, keep_versions)?;
    list_versions(core)
}

pub fn select_version(core: &SharedCore, req: SelectVersionRequest) -> Result<CoreVersionList> {
    let (versions_dir, local_binary, current_path) = {
        let guard = core.read().unwrap();
        (
            guard.path.mihomo_versions_dir.clone(),
            guard.path.mihomo_local_binary.clone(),
            guard.path.mihomo_current_binary.clone(),
        )
    };

    let (resolved, selected_version) = if req.version == "latest" {
        if let Some(path) = latest_version_file(&versions_dir) {
            let version = path
                .file_name()
                .and_then(|x| x.to_str())
                .and_then(extract_version)
                .unwrap_or_else(|| "latest".to_string());
            (path, version)
        } else if local_binary.exists() {
            (local_binary, "local".to_string())
        } else {
            return Err(anyhow!("没有可用内核版本"));
        }
    } else if req.version == "local" {
        (local_binary, "local".to_string())
    } else {
        (
            versions_dir.join(core_binary_file_name(&req.version)),
            req.version.clone(),
        )
    };

    if !resolved.exists() {
        return Err(anyhow!("目标内核版本不存在: {}", req.version));
    }

    relink_current(&current_path, &resolved)?;

    {
        let mut guard = core.write().unwrap();
        guard.config.mihomo_version = selected_version;
        guard.save_config();
    }

    list_versions(core)
}

pub fn set_auto_update(core: &SharedCore, req: AutoUpdateRequest) -> Result<CoreVersionList> {
    {
        let mut guard = core.write().unwrap();
        guard.config.core_auto_update = req.enabled;
        guard.save_config();
    }

    list_versions(core)
}

pub fn set_keep_versions(core: &SharedCore, req: KeepVersionsRequest) -> Result<CoreVersionList> {
    let keep_versions = req.keep_versions.max(1);
    {
        let mut guard = core.write().unwrap();
        guard.config.core_keep_versions = keep_versions;
        guard.save_config();
    }

    prune_versions(core, keep_versions)?;
    list_versions(core)
}

pub async fn sync_latest_from_github(core: &SharedCore) -> Result<GithubSyncResult> {
    let release: GithubRelease = reqwest::Client::new()
        .get(MIHOMO_GITHUB_LATEST_API)
        .header("User-Agent", "minihomo")
        .send()
        .await
        .context("请求 GitHub Release 失败")?
        .error_for_status()
        .context("GitHub Release 返回错误状态")?
        .json()
        .await
        .context("解析 GitHub Release 内容失败")?;

    let version_asset = release
        .assets
        .iter()
        .find(|x| x.name == "version.txt")
        .ok_or_else(|| anyhow!("未找到 version.txt"))?;

    let version_response = reqwest::get(&version_asset.browser_download_url)
        .await
        .context("下载 version.txt 失败")?
        .error_for_status()
        .context("version.txt 返回错误状态")?;

    let version = version_response
        .text()
        .await
        .context("读取 version.txt 失败")?
        .trim()
        .to_string();

    let asset = pick_platform_asset(&release.assets)
        .ok_or_else(|| anyhow!("未找到当前平台可用的 mihomo 资产"))?;

    let versions = install_core(
        core,
        InstallCoreRequest {
            version,
            url: asset.browser_download_url.clone(),
            set_current: true,
            source_tag: Some(release.tag_name.clone()),
            released_at: release.published_at.clone(),
        },
    )
    .await?;

    Ok(GithubSyncResult {
        tag: release.tag_name,
        asset_name: asset.name.clone(),
        asset_url: asset.browser_download_url.clone(),
        release_published_at: release.published_at,
        versions,
    })
}

pub async fn auto_update_if_enabled(core: &SharedCore) -> Result<Option<GithubSyncResult>> {
    let enabled = {
        let guard = core.read().unwrap();
        guard.config.core_auto_update
    };
    if !enabled {
        return Ok(None);
    }

    let synced = sync_latest_from_github(core).await?;
    Ok(Some(synced))
}

pub async fn ensure_bootstrap_core(core: &SharedCore) -> Result<()> {
    let versions = list_versions(core)?;
    if versions.versions.iter().any(|x| x.is_current) {
        return Ok(());
    }

    if versions.versions.is_empty() {
        let local_exists = {
            let guard = core.read().unwrap();
            guard.path.mihomo_local_binary.exists()
        };

        if local_exists {
            let _ = select_version(
                core,
                SelectVersionRequest {
                    version: "local".to_string(),
                },
            )?;
            return Ok(());
        }

        let _ = sync_latest_from_github(core).await?;
        return Ok(());
    }

    let latest_version = versions
        .versions
        .first()
        .map(|x| x.version.clone())
        .ok_or_else(|| anyhow!("没有可用内核版本"))?;
    let _ = select_version(
        core,
        SelectVersionRequest {
            version: latest_version,
        },
    )?;
    Ok(())
}

fn pick_platform_asset(assets: &[GithubAsset]) -> Option<&GithubAsset> {
    let mut candidates: Vec<&GithubAsset> = assets.iter().filter(|x| is_valid_asset(x)).collect();

    candidates.sort_by_key(|a| std::cmp::Reverse(a.download_count));
    candidates.into_iter().next()
}

#[cfg(target_os = "windows")]
fn is_valid_asset(asset: &GithubAsset) -> bool {
    (asset.name.contains("windows-amd64") || asset.name.contains("windows-x86_64"))
        && asset.name.ends_with(".zip")
}

#[cfg(target_os = "linux")]
fn is_valid_asset(asset: &GithubAsset) -> bool {
    let matches_arch = if cfg!(target_arch = "x86_64") {
        asset.name.contains("linux-amd64") || asset.name.contains("linux-x86_64")
    } else if cfg!(target_arch = "aarch64") {
        asset.name.contains("linux-arm64") || asset.name.contains("linux-aarch64")
    } else {
        false
    };
    matches_arch && asset.name.ends_with(".gz")
}

#[cfg(target_os = "macos")]
fn is_valid_asset(asset: &GithubAsset) -> bool {
    let matches_arch = if cfg!(target_arch = "aarch64") {
        asset.name.contains("darwin-arm64")
    } else {
        asset.name.contains("darwin-amd64")
    };
    matches_arch && asset.name.ends_with(".gz")
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn is_valid_asset(_asset: &GithubAsset) -> bool {
    false
}

fn write_core_binary(target_file: &Path, payload: &[u8]) -> Result<()> {
    let tmp_dir = target_file.parent().unwrap().join("tmp");
    fs::create_dir_all(&tmp_dir)
        .with_context(|| format!("创建临时目录失败: {}", tmp_dir.display()))?;

    let binary = extract_binary(&tmp_dir, payload).context("解压内核失败")?;

    let _ = fs::remove_dir_all(&tmp_dir);

    atomic_write(target_file, &binary)
}

#[cfg(target_os = "windows")]
fn extract_binary(tmp_dir: &Path, payload: &[u8]) -> Result<Vec<u8>> {
    use std::io::Read;
    use zip::ZipArchive;

    let zip_file = tmp_dir.join("archive.zip");
    fs::write(&zip_file, payload)
        .with_context(|| format!("写入 ZIP 文件失败: {}", zip_file.display()))?;

    let mut archive = ZipArchive::new(std::fs::File::open(&zip_file)?).context("解析 ZIP 失败")?;

    let mut selected_index: Option<usize> = None;

    for idx in 0..archive.len() {
        let file = archive.by_index(idx).context("读取 ZIP 条目失败")?;
        if file.name().to_ascii_lowercase().ends_with(".exe") {
            selected_index = Some(idx);
            break;
        }
    }

    if let Some(index) = selected_index {
        let mut file = archive.by_index(index).context("读取 ZIP 内核文件失败")?;
        let mut out = Vec::new();
        file.read_to_end(&mut out).context("读取 ZIP 内核失败")?;
        let _ = fs::remove_file(&zip_file);
        return Ok(out);
    }

    Err(anyhow!(
        "ZIP 内未找到可执行内核文件, {}",
        archive.file_names().collect::<Vec<_>>().join(", ")
    ))
}

#[cfg(not(target_os = "windows"))]
fn extract_binary(tmp_dir: &Path, payload: &[u8]) -> Result<Vec<u8>> {
    use std::process::Command;

    let gz_file = tmp_dir.join("archive.gz");
    fs::write(&gz_file, payload)
        .with_context(|| format!("写入 gz 文件失败: {}", gz_file.display()))?;

    Command::new("unzip")
        .arg("-q")
        .arg(&gz_file)
        .arg("-d")
        .arg(&tmp_dir)
        .output()
        .or_else(|_| {
            Command::new("tar")
                .arg("-xzf")
                .arg(&gz_file)
                .arg("-C")
                .arg(&tmp_dir)
                .output()
        })
        .context("解压 gz 文件失败")?;

    for entry in fs::read_dir(&tmp_dir).context("读取临时目录失败")? {
        let entry = entry?;
        let path = entry.path();
        let file_name = path.file_name().unwrap().to_string_lossy().to_lowercase();

        if (file_name == "mihomo" || file_name.starts_with("mihomo-"))
            && !file_name.ends_with(".txt")
        {
            let binary = fs::read(&path).context("读取二进制文件失败")?;
            let _ = fs::remove_file(&gz_file);
            return Ok(binary);
        }
    }

    Err(anyhow!("gz 内未找到可执行内核文件"))
}

fn atomic_write(target_file: &Path, bytes: &[u8]) -> Result<()> {
    let tmp_file = target_file.with_extension("tmp");
    fs::write(&tmp_file, bytes)
        .with_context(|| format!("写入临时内核文件失败: {}", tmp_file.display()))?;
    fs::rename(&tmp_file, target_file).with_context(|| {
        format!(
            "替换内核文件失败: {} -> {}",
            tmp_file.display(),
            target_file.display()
        )
    })?;
    Ok(())
}

fn prune_versions(core: &SharedCore, keep_versions: usize) -> Result<()> {
    let (versions_dir, current_file) = {
        let guard = core.read().unwrap();
        (
            guard.path.mihomo_versions_dir.clone(),
            fs::canonicalize(&guard.path.mihomo_current_binary).ok(),
        )
    };

    let mut entries: Vec<(PathBuf, SystemTime)> = fs::read_dir(&versions_dir)?
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

    if entries.len() <= keep_versions {
        return Ok(());
    }

    entries.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
    let mut kept = 0usize;
    for (path, _) in entries {
        let is_current = match (&current_file, fs::canonicalize(&path).ok()) {
            (Some(current), Some(candidate)) => current == &candidate,
            _ => false,
        };

        if kept < keep_versions || is_current {
            kept += 1;
            continue;
        }

        let _ = fs::remove_file(path.clone());
        let _ = fs::remove_file(meta_path_for(&path));
    }

    Ok(())
}

fn latest_version_file(versions_dir: &Path) -> Option<PathBuf> {
    let mut latest: Option<(PathBuf, SystemTime)> = None;

    for entry in fs::read_dir(versions_dir).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let modified = entry.metadata().ok()?.modified().ok()?;
        match &latest {
            Some((_, old)) if *old >= modified => {}
            _ => latest = Some((path, modified)),
        }
    }

    latest.map(|x| x.0)
}

fn relink_current(current: &Path, target: &Path) -> Result<()> {
    if current.exists() {
        fs::remove_file(current)
            .with_context(|| format!("删除旧内核链接失败: {}", current.display()))?;
    }

    #[cfg(target_os = "windows")]
    {
        if std::os::windows::fs::symlink_file(target, current).is_err() {
            fs::copy(target, current).with_context(|| {
                format!(
                    "创建 current 内核副本失败: {} -> {}",
                    target.display(),
                    current.display()
                )
            })?;
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        std::os::unix::fs::symlink(target, current).with_context(|| {
            format!(
                "创建 current 内核软链接失败: {} -> {}",
                target.display(),
                current.display()
            )
        })?;
    }

    Ok(())
}

fn linked_to_current(current: &Path, candidate: &Path) -> Result<bool> {
    let current_abs = fs::canonicalize(current).with_context(|| {
        format!(
            "读取 current 内核路径失败，请先选择可用版本: {}",
            current.display()
        )
    })?;
    let candidate_abs = fs::canonicalize(candidate)
        .with_context(|| format!("读取候选内核路径失败: {}", candidate.display()))?;
    Ok(current_abs == candidate_abs)
}

fn extract_version(file_name: &str) -> Option<String> {
    if cfg!(target_os = "windows") {
        let prefix = "mihomo-";
        let suffix = ".exe";
        if file_name.starts_with(prefix) && file_name.ends_with(suffix) {
            return Some(file_name[prefix.len()..file_name.len() - suffix.len()].to_string());
        }
    } else if let Some(rest) = file_name.strip_prefix("mihomo-") {
        return Some(rest.to_string());
    }

    None
}

fn build_version_info(
    current_bin: &Path,
    path: &Path,
    version: String,
    is_local: bool,
) -> CoreVersionInfo {
    let meta = read_meta_for(path);

    let downloaded_unix = meta
        .as_ref()
        .map(|x| x.downloaded_unix)
        .unwrap_or_else(|| file_modified_unix(path));

    let released_at = meta.as_ref().and_then(|x| x.released_at.clone());
    let source_tag = meta.as_ref().and_then(|x| x.source_tag.clone());

    CoreVersionInfo {
        version,
        file: path.display().to_string(),
        downloaded_unix,
        released_at,
        source_tag,
        is_current: linked_to_current(current_bin, path).unwrap_or(false),
        is_local,
    }
}

fn read_meta_for(path: &Path) -> Option<CoreVersionMeta> {
    let meta_file = meta_path_for(path);
    let content = fs::read_to_string(meta_file).ok()?;
    serde_json::from_str::<CoreVersionMeta>(&content).ok()
}

fn write_meta_for(path: &Path, meta: CoreVersionMeta) -> Result<()> {
    let meta_file = meta_path_for(path);
    let content = serde_json::to_string_pretty(&meta)?;
    fs::write(&meta_file, content)
        .with_context(|| format!("写入核心元数据失败: {}", meta_file.display()))?;
    Ok(())
}

fn meta_path_for(path: &Path) -> PathBuf {
    let mut p = path.to_path_buf();
    let ext = p
        .extension()
        .and_then(|x| x.to_str())
        .map(|x| format!("{x}.meta.json"))
        .unwrap_or_else(|| "meta.json".to_string());
    p.set_extension(ext);
    p
}

fn file_modified_unix(path: &Path) -> u64 {
    fs::metadata(path)
        .ok()
        .and_then(|x| x.modified().ok())
        .and_then(|x| x.duration_since(UNIX_EPOCH).ok())
        .map(|x| x.as_secs())
        .unwrap_or(0)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn is_http_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}
