use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    env,
    fs::File,
    io::BufReader,
    ops::Deref,
    path::{self, Path, PathBuf},
    sync::{Arc, RwLock},
};
use strum::Display;

pub struct Core {
    pub data_dir: PathBuf,
    pub config_path: PathBuf,

    pub config: Config,
    pub path: CorePath,
}

pub struct CorePath {
    pub temp_dir: PathBuf,
    pub runtime_config: PathBuf,
    pub override_config: PathBuf,
    pub subscription_dir: PathBuf,
    pub subscription_local_file: PathBuf,

    pub mihomo_dir: PathBuf,
    pub mihomo_versions_dir: PathBuf,
    pub mihomo_local_binary: PathBuf,
    pub mihomo_current_binary: PathBuf,
    pub mihomo_pid_file: PathBuf,
    pub mihomo_socker: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize, Display)]
pub enum PanelType {
    Zashboard,
    Metacubexd,
    YacdMeta,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SubscriptionItem {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub url: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default = "config_default_host")]
    pub host: String,
    #[serde(default = "config_default_port")]
    pub port: u16,
    #[serde(default)]
    pub remote_url: String,
    #[serde(default = "config_default_panel")]
    pub panel: PanelType,
    pub panel_version: Option<String>,

    #[serde(default)]
    pub subscription_items: Vec<SubscriptionItem>,
    #[serde(default = "config_default_interval_minutes")]
    pub interval_minutes: u64,
    #[serde(default = "config_default_subscription_keep_versions")]
    pub subscription_keep_versions: usize,
    #[serde(default = "config_default_core_auto_update")]
    pub core_auto_update: bool,
    #[serde(default = "config_default_core_keep_versions")]
    pub core_keep_versions: usize,
    #[serde(default = "config_default_version")]
    pub mihomo_version: String,
}

const fn config_default_panel() -> PanelType {
    PanelType::Zashboard
}

fn config_default_version() -> String {
    "latest".to_string()
}

const fn config_default_core_auto_update() -> bool {
    false
}

const fn config_default_core_keep_versions() -> usize {
    3
}

const fn config_default_subscription_keep_versions() -> usize {
    3
}

const fn config_default_port() -> u16 {
    12125
}

const fn config_default_interval_minutes() -> u64 {
    30
}

fn config_default_host() -> String {
    "127.0.0.1".to_string()
}

fn display_host(host: &str) -> &str {
    if host == "0.0.0.0" { "127.0.0.1" } else { host }
}

impl Core {
    pub fn new() -> SharedCore {
        let args_map: HashMap<String, String> = env::args()
            .skip(1)
            .filter_map(|arg| {
                let parts: Vec<&str> = arg.splitn(2, '=').collect();
                if parts.len() == 2 && parts[0].starts_with("--") {
                    Some((parts[0][2..].to_string(), parts[1].to_string()))
                } else {
                    None
                }
            })
            .collect();

        let data_dir = {
            if let Some(path) = args_map.get("path") {
                path::absolute(path).ok()
            } else if let Ok(path) = env::var("MINIHOME_PATH") {
                path::absolute(path).ok()
            } else {
                dirs::config_dir().map(|mut path| {
                    path.push("minihomo");
                    path
                })
            }
        }
        .expect("无法获取配置路径");

        if !data_dir.exists() {
            std::fs::create_dir_all(&data_dir).expect("无法创建配置文件夹");
        }

        let config_path = data_dir.join("config.json");
        let mut config = Core::load_config(&config_path).expect("无法加载配置文件");
        if config.remote_url.trim().is_empty() {
            config.remote_url = format!("http://{}:{}", display_host(&config.host), config.port);
        }

        let mconfig_path = data_dir.join("mconfig");
        let temp_dir = mconfig_path.join("tmp");

        let mihomo_dir = data_dir.join("mihomo");
        let mihomo_bin_dir = mihomo_dir.join("bin");
        let subscription_dir = mconfig_path.join("subscriptions");

        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).expect("无法创建运行配置路径");
        std::fs::create_dir_all(&mihomo_dir).expect("无法创建mihomo文件路径");
        std::fs::create_dir_all(&mihomo_bin_dir).expect("无法创建mihomo内核目录");
        std::fs::create_dir_all(&subscription_dir).expect("无法创建订阅目录");

        let override_config = mconfig_path.join("override.yaml");
        let runtime_config = temp_dir.join("runtime.yaml");
        let subscription_local_file = subscription_dir.join("local").join("latest.yaml");

        let core = Core {
            config_path,
            config,
            path: CorePath {
                temp_dir,
                runtime_config,
                override_config,
                subscription_dir,
                subscription_local_file,

                mihomo_pid_file: mihomo_dir.join("core.pid"),
                mihomo_socker: mihomo_dir.join("core.sock"),
                mihomo_versions_dir: mihomo_bin_dir.join("versions"),
                mihomo_local_binary: mihomo_bin_dir.join(local_core_binary_name()),
                mihomo_current_binary: mihomo_bin_dir.join(core_binary_file_name("current")),
                mihomo_dir,
            },
            data_dir,
        };

        core.ensure_runtime_files();
        core.save_config();
        SharedCore::new(Arc::new(RwLock::new(core))).expect("核心配置初始化失败")
    }

    fn ensure_runtime_files(&self) {
        let _ = std::fs::create_dir_all(&self.path.mihomo_versions_dir);
        let _ = std::fs::create_dir_all(&self.path.subscription_dir);

        ensure_file_with_default(&self.path.override_config, "# 覆写配置\n");
        ensure_file_with_default(
            &self.path.subscription_local_file,
            default_local_subscription(),
        );

        let legacy_local = self.path.subscription_dir.join("local.yaml");
        if legacy_local.exists() && !self.path.subscription_local_file.exists() {
            if let Some(parent) = self.path.subscription_local_file.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ =
                std::fs::rename(&legacy_local, &self.path.subscription_local_file).or_else(|_| {
                    std::fs::copy(&legacy_local, &self.path.subscription_local_file).map(|_| ())
                });
        }
    }

    pub fn save_config(&self) {
        let file = File::create(&self.config_path).expect("无法创建config.json配置文件");
        serde_json::to_writer_pretty(file, &self.config).expect("无法写入config.json配置文件");
    }

    fn load_config(config_path: &Path) -> Result<Config> {
        if config_path.exists() {
            let file = File::open(config_path)?;
            let reader = BufReader::new(file);
            let config = serde_json::from_reader(reader)?;
            Ok(config)
        } else {
            Ok(serde_json::from_str("{}")?)
        }
    }
}

#[derive(Clone)]
pub struct SharedCore {
    inner: Arc<RwLock<Core>>,
}

impl Deref for SharedCore {
    type Target = Arc<RwLock<Core>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl SharedCore {
    pub fn new(inner: Arc<RwLock<Core>>) -> Result<Self> {
        Ok(SharedCore { inner })
    }

    pub fn get_panel(&self) -> PathBuf {
        let (data_dir, panel, version) = {
            let core = self.read().unwrap();
            (
                core.data_dir.clone(),
                core.config.panel.clone(),
                core.config.panel_version.clone(),
            )
        };

        let panel_root = data_dir.join("panel").join(panel.to_string());
        if let Some(panel_version) = version.or_else(|| discover_panel_version(&panel_root)) {
            panel_root.join(panel_version)
        } else {
            panel_root
        }
    }
}

pub fn core_binary_file_name(version: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("mihomo-{version}.exe")
    } else {
        format!("mihomo-{version}")
    }
}

pub fn local_core_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "mihomo.exe"
    } else {
        "mihomo"
    }
}

fn ensure_file_with_default(path: &Path, default_content: &str) {
    if path.exists() {
        return;
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, default_content);
}

fn discover_panel_version(panel_root: &Path) -> Option<String> {
    let mut versions: Vec<String> = std::fs::read_dir(panel_root)
        .ok()?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.path().is_dir() {
                return None;
            }

            entry.file_name().to_str().map(|x| x.to_string())
        })
        .collect();

    versions.sort();
    versions.pop()
}

fn default_mihomo_source() -> &'static str {
    "mixed-port: 7890\nmode: rule\nlog-level: info\nrules:\n  - MATCH,DIRECT\n"
}

fn default_local_subscription() -> &'static str {
    "# 本地订阅配置\nmixed-port: 7890\nmode: rule\nlog-level: info\n"
}
