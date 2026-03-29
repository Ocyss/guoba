use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use std::{
    fs::File,
    path::PathBuf,
    process::{Command, Stdio},
};

use crate::core::SharedCore;
use crate::{subscription, yaml_overlay};

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeRenderInfo {
    pub source_path: String,
    pub override_path: String,
    pub rendered_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CoreStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub executable: Option<String>,
    pub runtime_config: String,
}

pub fn get_status(core: &SharedCore) -> Result<CoreStatus> {
    let (pid_file, runtime_config) = {
        let guard = core.read().unwrap();
        (
            guard.path.mihomo_pid_file.clone(),
            guard.path.runtime_config.clone(),
        )
    };
    let pid = read_pid(&pid_file);
    let running = pid.is_some_and(is_process_running);

    Ok(CoreStatus {
        running,
        pid,
        // executable: executable.map(|x| x.display().to_string()),
        executable: None,
        runtime_config: runtime_config.display().to_string(),
    })
}

pub fn restart(core: &SharedCore) -> Result<CoreStatus> {
    let runtime_render = render_runtime_config(core, None, None)?;

    let (working_dir, pid_file, socker, runtime_config, executable) = {
        let guard = core.read().unwrap();
        (
            guard.path.mihomo_dir.clone(),
            guard.path.mihomo_pid_file.clone(),
            guard.path.mihomo_socker.clone(),
            guard.path.runtime_config.clone(),
            guard.path.mihomo_current_binary.clone(),
        )
    };

    if !executable.exists() {
        return Err(anyhow!("当前内核不存在: {}", executable.display()));
    }

    if let Some(pid) = read_pid(&pid_file) {
        if is_process_running(pid) {
            kill_process(pid).with_context(|| format!("停止旧进程失败: {pid}"))?;
        }
    }
    let log_file = File::create(working_dir.join("output.log"))?;
    let error_file = File::create(working_dir.join("error.log"))?;

    let mut command = Command::new(&executable);
    command
        .arg("-d")
        .arg(&working_dir)
        .arg("-f")
        .arg(&runtime_config)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(error_file));

    // #[cfg(target_os = "windows")]
    // let command = command.arg("-ext-ctl-pipe").arg(&socker);
    // #[cfg(not(target_os = "windows"))]
    let command = command.arg("-ext-ctl-unix").arg(&socker);

    let child = command
        .spawn()
        .with_context(|| format!("启动内核失败: {}", executable.display()))?;

    log::info!(
        "运行配置已渲染: source={}, override={}, rendered={}",
        runtime_render.source_path,
        runtime_render.override_path,
        runtime_render.rendered_path
    );

    let pid = child.id();
    std::fs::write(&pid_file, pid.to_string())
        .with_context(|| format!("写入 pid 文件失败: {}", pid_file.display()))?;

    get_status(core)
}

pub async fn hot_reload(core: &SharedCore) -> Result<()> {
    let _ = restart(core)?;
    Ok(())
}

pub fn render_runtime_config(
    core: &SharedCore,
    source_content: Option<&str>,
    source_hint: Option<&str>,
) -> Result<RuntimeRenderInfo> {
    let (override_path, runtime_path, local_source_path) = {
        let guard = core.read().unwrap();
        (
            guard.path.override_config.clone(),
            guard.path.runtime_config.clone(),
            guard.path.subscription_local_file.clone(),
        )
    };

    let override_content = std::fs::read_to_string(&override_path)
        .with_context(|| format!("读取覆写配置失败: {}", override_path.display()))?;

    let (source_path, source_yaml) = match source_content {
        Some(content) => (
            source_hint.unwrap_or("<request>").to_string(),
            content.to_string(),
        ),
        None => {
            let view = subscription::get_subscription_view(core)?;
            if view.default_id != view.local_id {
                let content = subscription::read_subscription_content(
                    core,
                    subscription::SubscriptionContentRequest {
                        id: Some(view.default_id.clone()),
                        name: None,
                    },
                )?;
                (content.path, content.content)
            } else {
                (
                    local_source_path.display().to_string(),
                    std::fs::read_to_string(&local_source_path).unwrap_or_default(),
                )
            }
        }
    };

    let rendered = yaml_overlay::render_yaml(&source_yaml, &override_content)?;

    if let Some(parent) = runtime_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建运行配置目录失败: {}", parent.display()))?;
    }
    std::fs::write(&runtime_path, rendered)
        .with_context(|| format!("写入运行配置失败: {}", runtime_path.display()))?;

    Ok(RuntimeRenderInfo {
        source_path,
        override_path: override_path.display().to_string(),
        rendered_path: runtime_path.display().to_string(),
    })
}

pub fn render_preview_yaml(
    core: &SharedCore,
    source_content: Option<&str>,
    override_content: Option<&str>,
) -> Result<String> {
    let override_yaml = match override_content {
        Some(content) => content.to_string(),
        None => {
            let path = {
                let guard = core.read().unwrap();
                guard.path.override_config.clone()
            };
            std::fs::read_to_string(&path)
                .with_context(|| format!("读取覆写配置失败: {}", path.display()))?
        }
    };

    let source_yaml = match source_content {
        Some(content) => content.to_string(),
        None => {
            let path = {
                let guard = core.read().unwrap();
                guard.path.subscription_local_file.clone()
            };
            std::fs::read_to_string(&path).unwrap_or_default()
        }
    };

    yaml_overlay::render_yaml(&source_yaml, &override_yaml)
}

fn read_pid(pid_file: &PathBuf) -> Option<u32> {
    let content = std::fs::read_to_string(pid_file).ok()?;
    content.trim().parse::<u32>().ok()
}

#[cfg(target_os = "windows")]
fn is_process_running(pid: u32) -> bool {
    let filter = format!("PID eq {pid}");
    let output = Command::new("tasklist")
        .args(["/FI", &filter, "/NH"])
        .output();

    match output {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stdout);
            text.contains(&pid.to_string())
        }
        Err(_) => false,
    }
}

#[cfg(not(target_os = "windows"))]
fn is_process_running(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "windows")]
fn kill_process(pid: u32) -> Result<()> {
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()
        .context("执行 taskkill 失败")?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("taskkill 返回非零状态"))
    }
}

#[cfg(not(target_os = "windows"))]
fn kill_process(pid: u32) -> Result<()> {
    let status = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .context("执行 kill 失败")?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("kill 返回非零状态"))
    }
}
