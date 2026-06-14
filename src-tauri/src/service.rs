use std::{collections::HashMap, sync::Arc, thread, time::Duration};

use anyhow::{Context, Result, bail};
use parking_lot::RwLock;
use serde::Serialize;

use crate::{
    config::{ActionKind, ActionWhen, LoadedConfig, ServiceConfig, ServiceType, service_log_path},
    logs,
    process_manager::ProcessManager,
    terminal, windows_service,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceLifecycle {
    Stopped,
    Starting,
    Running,
    Stopping,
    Restarting,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeServiceState {
    pub lifecycle: ServiceLifecycle,
    pub detail: Option<String>,
    pub pid: Option<u32>,
    pub last_error: Option<String>,
}

impl RuntimeServiceState {
    pub fn new(lifecycle: ServiceLifecycle) -> Self {
        Self {
            lifecycle,
            detail: None,
            pid: None,
            last_error: None,
        }
    }
}

impl Default for RuntimeServiceState {
    fn default() -> Self {
        Self::new(ServiceLifecycle::Stopped)
    }
}

pub struct ServiceRegistry {
    config: Arc<RwLock<LoadedConfig>>,
    states: Arc<RwLock<HashMap<String, RuntimeServiceState>>>,
    processes: ProcessManager,
}

impl ServiceRegistry {
    pub fn new(config: LoadedConfig) -> Self {
        let states = Arc::new(RwLock::new(
            config
                .config
                .services
                .iter()
                .map(|service| (service.id.clone(), RuntimeServiceState::default()))
                .collect(),
        ));
        let processes = ProcessManager::new(states.clone());
        Self {
            config: Arc::new(RwLock::new(config)),
            states,
            processes,
        }
    }

    pub fn config(&self) -> LoadedConfig {
        self.config.read().clone()
    }

    pub fn set_refresh_callback(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        self.processes.set_refresh_callback(callback);
    }

    pub fn reload_config(&self, loaded: LoadedConfig) {
        let mut states = self.states.write();
        states.retain(|id, _| {
            loaded
                .config
                .services
                .iter()
                .any(|service| &service.id == id)
        });
        for service in &loaded.config.services {
            states.entry(service.id.clone()).or_default();
        }
        *self.config.write() = loaded;
    }

    pub fn get_service_state(&self, service_id: &str) -> RuntimeServiceState {
        let service = self.find_service(service_id);
        if let Ok(service) = service
            && service.service_type == ServiceType::WindowsService
        {
            return self.query_windows_service(&service);
        }
        self.states
            .read()
            .get(service_id)
            .cloned()
            .unwrap_or_else(|| RuntimeServiceState::new(ServiceLifecycle::Unknown))
    }

    pub fn start_service(&self, service_id: &str) -> Result<()> {
        let service = self.find_service(service_id)?;
        match service.service_type {
            ServiceType::Process | ServiceType::ReactNative => {
                let loaded = self.config.read();
                let log_path = service_log_path(&service, &loaded.log_dir);
                self.processes.start(&service, &log_path, None)
            }
            ServiceType::WindowsService => {
                let name = windows_service_name(&service)?;
                self.set_lifecycle(service_id, ServiceLifecycle::Starting);
                windows_service::start(name)?;
                self.set_lifecycle(service_id, ServiceLifecycle::Running);
                Ok(())
            }
        }
    }

    pub fn stop_service(&self, service_id: &str) -> Result<()> {
        let service = self.find_service(service_id)?;
        match service.service_type {
            ServiceType::Process | ServiceType::ReactNative => self.processes.stop(&service),
            ServiceType::WindowsService => {
                let name = windows_service_name(&service)?;
                self.set_lifecycle(service_id, ServiceLifecycle::Stopping);
                windows_service::stop(name)?;
                self.set_lifecycle(service_id, ServiceLifecycle::Stopped);
                Ok(())
            }
        }
    }

    pub fn restart_service(&self, service_id: &str) -> Result<()> {
        let service = self.find_service(service_id)?;
        self.set_lifecycle(service_id, ServiceLifecycle::Restarting);
        match service.service_type {
            ServiceType::Process | ServiceType::ReactNative => {
                if self.processes.is_running(service_id) {
                    self.processes.stop(&service)?;
                    self.wait_until_stopped(service_id, Duration::from_secs(10))?;
                }
                thread::sleep(Duration::from_millis(service.process.restart_delay_ms));
                let loaded = self.config.read();
                let log_path = service_log_path(&service, &loaded.log_dir);
                self.processes.start(&service, &log_path, None)
            }
            ServiceType::WindowsService => {
                let name = windows_service_name(&service)?;
                if windows_service::query(name)? != ServiceLifecycle::Stopped {
                    windows_service::stop(name)?;
                    thread::sleep(Duration::from_millis(service.process.restart_delay_ms));
                }
                windows_service::start(name)?;
                self.set_lifecycle(service_id, ServiceLifecycle::Running);
                Ok(())
            }
        }
    }

    pub fn run_action(&self, service_id: &str, action_id: &str) -> Result<()> {
        let service = self.find_service(service_id)?;
        let action = service
            .actions
            .iter()
            .find(|action| action.id == action_id)
            .with_context(|| format!("action '{action_id}' does not exist"))?;
        let state = self.get_service_state(service_id);
        if !action_is_available(action.when, state.lifecycle) {
            bail!(
                "action '{action_id}' is not available while service is {:?}",
                state.lifecycle
            );
        }

        match action.kind {
            ActionKind::Stdin => self.processes.write_stdin(
                service_id,
                action
                    .input
                    .as_deref()
                    .context("stdin action has no input")?,
            ),
            ActionKind::Restart => {
                self.set_lifecycle(service_id, ServiceLifecycle::Restarting);
                if self.processes.is_running(service_id) {
                    self.processes.stop(&service)?;
                    self.wait_until_stopped(service_id, Duration::from_secs(10))?;
                }
                thread::sleep(Duration::from_millis(service.process.restart_delay_ms));
                let loaded = self.config.read();
                let log_path = service_log_path(&service, &loaded.log_dir);
                match action.command.as_deref() {
                    Some(command) => {
                        self.processes
                            .start(&service, &log_path, Some((command, &action.args)))
                    }
                    None => self.processes.start(&service, &log_path, None),
                }
            }
        }
    }

    pub fn run_react_native_command(&self, service_id: &str, input: &str) -> Result<()> {
        let service = self.find_service(service_id)?;
        if service.service_type != ServiceType::ReactNative {
            bail!("service '{service_id}' is not a react-native service");
        }

        let state = self.get_service_state(service_id);
        if state.lifecycle != ServiceLifecycle::Running {
            bail!(
                "react-native command is not available while service is {:?}",
                state.lifecycle
            );
        }

        self.processes.write_stdin(service_id, input)
    }

    pub fn open_log(&self, service_id: &str) -> Result<()> {
        let service = self.find_service(service_id)?;
        let loaded = self.config.read();
        logs::open_log(&service_log_path(&service, &loaded.log_dir))
    }

    pub fn open_log_in_terminal(&self, service_id: &str) -> Result<()> {
        let service = self.find_service(service_id)?;
        let loaded = self.config.read();
        let log_path = service_log_path(&service, &loaded.log_dir);
        logs::ensure_log_file(&log_path)?;
        terminal::open_log_in_terminal(&loaded.config.app.terminal, &service.name, &log_path)
    }

    pub fn start_launch_services(&self) {
        let services = self.config.read().config.services.clone();
        for service in services {
            if service.process.start_on_launch
                && let Err(error) = self.start_service(&service.id)
            {
                self.record_error(&service.id, error.to_string());
            }
        }
    }

    pub fn stop_all(&self) {
        let services = self.config.read().config.services.clone();
        self.processes.stop_all(&services);
    }

    pub fn record_error(&self, service_id: &str, message: String) {
        self.states.write().insert(
            service_id.to_string(),
            RuntimeServiceState {
                lifecycle: ServiceLifecycle::Failed,
                detail: Some("error".to_string()),
                pid: None,
                last_error: Some(message),
            },
        );
    }

    fn find_service(&self, service_id: &str) -> Result<ServiceConfig> {
        self.config
            .read()
            .config
            .services
            .iter()
            .find(|service| service.id == service_id)
            .cloned()
            .with_context(|| format!("service '{service_id}' does not exist"))
    }

    fn query_windows_service(&self, service: &ServiceConfig) -> RuntimeServiceState {
        match windows_service_name(service).and_then(windows_service::query) {
            Ok(lifecycle) => {
                let state = RuntimeServiceState::new(lifecycle);
                self.states
                    .write()
                    .insert(service.id.clone(), state.clone());
                state
            }
            Err(error) => RuntimeServiceState {
                lifecycle: ServiceLifecycle::Failed,
                detail: Some("error".to_string()),
                pid: None,
                last_error: Some(error.to_string()),
            },
        }
    }

    fn wait_until_stopped(&self, service_id: &str, timeout: Duration) -> Result<()> {
        let started = std::time::Instant::now();
        while self.processes.is_running(service_id) {
            if started.elapsed() >= timeout {
                bail!("timed out waiting for service '{service_id}' to stop");
            }
            thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }

    fn set_lifecycle(&self, service_id: &str, lifecycle: ServiceLifecycle) {
        let mut states = self.states.write();
        states.entry(service_id.to_string()).or_default().lifecycle = lifecycle;
    }
}

pub fn action_is_available(when: ActionWhen, lifecycle: ServiceLifecycle) -> bool {
    match when {
        ActionWhen::Any => true,
        ActionWhen::Running => lifecycle == ServiceLifecycle::Running,
        ActionWhen::Failed => lifecycle == ServiceLifecycle::Failed,
        ActionWhen::Stopped => lifecycle == ServiceLifecycle::Stopped,
    }
}

fn windows_service_name(service: &ServiceConfig) -> Result<&str> {
    service
        .windows_service
        .as_ref()
        .map(|options| options.service_name.as_str())
        .context("windowsService.serviceName is missing")
}
