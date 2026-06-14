use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevDockConfig {
    pub version: u32,
    #[serde(default)]
    pub app: AppConfig,
    #[serde(default)]
    pub groups: Vec<GroupConfig>,
    #[serde(default)]
    pub services: Vec<ServiceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    #[serde(default = "default_app_name")]
    pub name: String,
    #[serde(default = "default_log_dir_template")]
    pub log_dir: String,
    #[serde(default)]
    pub terminal: TerminalConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            name: default_app_name(),
            log_dir: default_log_dir_template(),
            terminal: TerminalConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalConfig {
    #[serde(default = "default_terminal_program")]
    pub program: String,
    #[serde(default = "default_terminal_shell")]
    pub shell: String,
    #[serde(default = "default_tail_command")]
    pub tail_command: String,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            program: default_terminal_program(),
            shell: default_terminal_shell(),
            tail_command: default_tail_command(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupConfig {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceConfig {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub service_type: ServiceType,
    pub group: String,
    pub cwd: Option<String>,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub process: ProcessOptions,
    pub windows_service: Option<WindowsServiceOptions>,
    pub log: Option<LogOptions>,
    pub status: Option<StatusOptions>,
    #[serde(default)]
    pub actions: Vec<ActionConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceType {
    Process,
    ReactNative,
    WindowsService,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessOptions {
    #[serde(default = "default_true")]
    pub kill_tree: bool,
    #[serde(default = "default_restart_delay")]
    pub restart_delay_ms: u64,
    #[serde(default)]
    pub start_on_launch: bool,
    pub keep_stdin: Option<bool>,
}

impl Default for ProcessOptions {
    fn default() -> Self {
        Self {
            kill_tree: true,
            restart_delay_ms: default_restart_delay(),
            start_on_launch: false,
            keep_stdin: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsServiceOptions {
    pub service_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogOptions {
    pub file: Option<String>,
    #[serde(default = "default_true")]
    pub rotate: bool,
    #[serde(default = "default_log_max_size_mb")]
    pub max_size_mb: Option<u64>,
    #[serde(default = "default_log_max_files")]
    pub max_files: Option<u32>,
}

impl Default for LogOptions {
    fn default() -> Self {
        Self {
            file: None,
            rotate: true,
            max_size_mb: default_log_max_size_mb(),
            max_files: default_log_max_files(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusOptions {
    pub mode: Option<String>,
    #[serde(default)]
    pub patterns: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionConfig {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub when: ActionWhen,
    pub kind: ActionKind,
    pub input: Option<String>,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionKind {
    Stdin,
    Restart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ActionWhen {
    Running,
    Failed,
    Stopped,
    #[default]
    Any,
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: DevDockConfig,
    pub config_path: PathBuf,
    pub log_dir: PathBuf,
}

pub fn load_or_create() -> Result<LoadedConfig> {
    let config_path = paths::config_path()?;
    if !config_path.exists() {
        create_example_config(&config_path)?;
    }
    load_from_path(&config_path)
}

pub fn load_from_path(config_path: &Path) -> Result<LoadedConfig> {
    let raw = fs::read_to_string(config_path)
        .with_context(|| format!("failed to read config {}", config_path.display()))?;
    let mut config: DevDockConfig = serde_json::from_str(&raw)
        .with_context(|| format!("invalid JSON in {}", config_path.display()))?;
    validate(&config)?;

    let app_data = paths::app_data_dir()?;
    let config_dir = config_path
        .parent()
        .context("config path has no parent directory")?
        .to_path_buf();
    let log_dir_value = expand_basic(&config.app.log_dir, &app_data, &config_dir);
    let log_dir = PathBuf::from(log_dir_value);

    config.app.log_dir = log_dir.to_string_lossy().into_owned();
    for service in &mut config.services {
        expand_service(service, &app_data, &config_dir, &log_dir);
    }

    fs::create_dir_all(&log_dir)
        .with_context(|| format!("failed to create log directory {}", log_dir.display()))?;

    Ok(LoadedConfig {
        config,
        config_path: config_path.to_path_buf(),
        log_dir,
    })
}

pub fn validate(config: &DevDockConfig) -> Result<()> {
    if config.version != 1 {
        bail!("unsupported config version {}; expected 1", config.version);
    }

    let mut group_ids = HashSet::new();
    for (index, group) in config.groups.iter().enumerate() {
        if group.id.trim().is_empty() {
            bail!("groups[{index}].id is required");
        }
        if !group_ids.insert(group.id.as_str()) {
            bail!("groups[{index}].id duplicates '{}'", group.id);
        }
    }

    let mut service_ids = HashSet::new();
    for (index, service) in config.services.iter().enumerate() {
        if service.id.trim().is_empty() {
            bail!("services[{index}].id is required");
        }
        if !service_ids.insert(service.id.as_str()) {
            bail!("services[{index}].id duplicates '{}'", service.id);
        }
        if !group_ids.contains(service.group.as_str()) {
            bail!("services[{index}].group '{}' does not exist", service.group);
        }

        match service.service_type {
            ServiceType::Process | ServiceType::ReactNative => {
                if service.command.as_deref().is_none_or(str::is_empty) {
                    bail!("services[{index}].command is required for type process");
                }
            }
            ServiceType::WindowsService => {
                if service
                    .windows_service
                    .as_ref()
                    .is_none_or(|options| options.service_name.trim().is_empty())
                {
                    bail!(
                        "services[{index}].windowsService.serviceName is required for type windows-service"
                    );
                }
            }
        }

        for (action_index, action) in service.actions.iter().enumerate() {
            if action.kind == ActionKind::Stdin {
                if action.input.is_none() {
                    bail!("services[{index}].actions[{action_index}].kind=stdin requires input");
                }
                if !service_keeps_stdin(service) {
                    bail!(
                        "services[{index}].actions[{action_index}].kind=stdin requires process stdin support"
                    );
                }
            }
        }
    }
    Ok(())
}

pub fn service_keeps_stdin(service: &ServiceConfig) -> bool {
    service
        .process
        .keep_stdin
        .unwrap_or(service.service_type == ServiceType::ReactNative)
}

pub fn service_log_path(service: &ServiceConfig, log_dir: &Path) -> PathBuf {
    service
        .log
        .as_ref()
        .and_then(|log| log.file.as_ref())
        .map(PathBuf::from)
        .unwrap_or_else(|| log_dir.join(format!("{}.log", service.id)))
}

fn expand_service(service: &mut ServiceConfig, app_data: &Path, config_dir: &Path, log_dir: &Path) {
    service.cwd = service
        .cwd
        .as_deref()
        .map(|value| expand_all(value, app_data, config_dir, log_dir));
    service.command = service
        .command
        .as_deref()
        .map(|value| expand_all(value, app_data, config_dir, log_dir));
    service.args = service
        .args
        .iter()
        .map(|value| expand_all(value, app_data, config_dir, log_dir))
        .collect();
    service.env = service
        .env
        .iter()
        .map(|(key, value)| {
            (
                key.clone(),
                expand_all(value, app_data, config_dir, log_dir),
            )
        })
        .collect();

    let default_log_file = log_dir.join(format!("{}.log", service.id));
    let log = service.log.get_or_insert_with(LogOptions::default);
    log.file = Some(
        log.file
            .as_deref()
            .map(|value| expand_all(value, app_data, config_dir, log_dir))
            .unwrap_or_else(|| default_log_file.to_string_lossy().into_owned()),
    );
    log.max_size_mb.get_or_insert(10);
    log.max_files.get_or_insert(1);
}

fn expand_basic(value: &str, app_data: &Path, config_dir: &Path) -> String {
    value
        .replace("${APP_DATA}", &app_data.to_string_lossy())
        .replace("${CONFIG_DIR}", &config_dir.to_string_lossy())
}

fn expand_all(value: &str, app_data: &Path, config_dir: &Path, log_dir: &Path) -> String {
    expand_basic(value, app_data, config_dir).replace("${LOG_DIR}", &log_dir.to_string_lossy())
}

fn create_example_config(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }
    fs::write(path, EXAMPLE_CONFIG)
        .with_context(|| format!("failed to create example config {}", path.display()))
}

fn default_true() -> bool {
    true
}

fn default_restart_delay() -> u64 {
    1000
}

fn default_log_max_size_mb() -> Option<u64> {
    Some(10)
}

fn default_log_max_files() -> Option<u32> {
    Some(1)
}

fn default_app_name() -> String {
    "DevDock".to_string()
}

fn default_log_dir_template() -> String {
    "${APP_DATA}/DevDock/logs".to_string()
}

fn default_terminal_program() -> String {
    "wt.exe".to_string()
}

fn default_terminal_shell() -> String {
    "powershell".to_string()
}

fn default_tail_command() -> String {
    "Get-Content -Path '{logFile}' -Wait".to_string()
}

const EXAMPLE_CONFIG: &str = r#"{
  "version": 1,
  "app": {
    "name": "DevDock",
    "logDir": "${APP_DATA}/DevDock/logs",
    "terminal": {
      "program": "wt.exe",
      "shell": "powershell",
      "tailCommand": "Get-Content -Path '{logFile}' -Wait"
    }
  },
  "groups": [
    { "id": "examples", "name": "Examples" }
  ],
  "services": [
    {
      "id": "example-server",
      "name": "Example Server",
      "type": "process",
      "group": "examples",
      "command": "powershell",
      "args": ["-NoProfile", "-Command", "while ($true) { Write-Output ('DevDock example ' + (Get-Date)); Start-Sleep 2 }"],
      "process": {
        "killTree": true,
        "restartDelayMs": 1000,
        "startOnLaunch": false
      },
      "log": {
        "file": "${LOG_DIR}/example-server.log",
        "rotate": true,
        "maxSizeMb": 10,
        "maxFiles": 1
      },
      "actions": []
    }
  ]
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_is_valid() {
        let config: DevDockConfig = serde_json::from_str(EXAMPLE_CONFIG).unwrap();
        validate(&config).unwrap();
    }

    #[test]
    fn react_native_keeps_stdin_by_default() {
        let config: DevDockConfig = serde_json::from_str(
            r#"{
              "version": 1,
              "groups": [{"id":"mobile","name":"Mobile"}],
              "services": [{
                "id":"metro",
                "name":"Metro",
                "type":"react-native",
                "group":"mobile",
                "command":"npx",
                "actions":[{"id":"reload","label":"Reload","kind":"stdin","input":"r"}]
              }]
            }"#,
        )
        .unwrap();
        validate(&config).unwrap();
    }

    #[test]
    fn missing_log_uses_service_defaults() {
        let mut service: ServiceConfig = serde_json::from_str(
            r#"{
              "id":"api",
              "name":"API",
              "type":"process",
              "group":"backend",
              "command":"api.exe"
            }"#,
        )
        .unwrap();
        let log_dir = Path::new(r"C:\logs");

        expand_service(
            &mut service,
            Path::new(r"C:\app-data"),
            Path::new(r"C:\config"),
            log_dir,
        );

        let log = service.log.unwrap();
        assert_eq!(log.file.as_deref(), Some(r"C:\logs\api.log"));
        assert!(log.rotate);
        assert_eq!(log.max_size_mb, Some(10));
        assert_eq!(log.max_files, Some(1));
    }

    #[test]
    fn partial_log_uses_field_defaults() {
        let mut service: ServiceConfig = serde_json::from_str(
            r#"{
              "id":"api",
              "name":"API",
              "type":"process",
              "group":"backend",
              "command":"api.exe",
              "log":{"file":"${LOG_DIR}/custom.log"}
            }"#,
        )
        .unwrap();
        let log_dir = Path::new(r"C:\logs");

        expand_service(
            &mut service,
            Path::new(r"C:\app-data"),
            Path::new(r"C:\config"),
            log_dir,
        );

        let log = service.log.unwrap();
        assert_eq!(log.file.as_deref(), Some(r"C:\logs/custom.log"));
        assert!(log.rotate);
        assert_eq!(log.max_size_mb, Some(10));
        assert_eq!(log.max_files, Some(1));
    }
}
