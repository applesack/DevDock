use std::{
    collections::HashMap,
    process::{Child, ChildStdin, Command, Stdio},
    sync::Arc,
    thread,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use parking_lot::{Mutex, RwLock};

use crate::{
    config::{ServiceConfig, StatusOptions, service_keeps_stdin},
    logs,
    service::{RuntimeServiceState, ServiceLifecycle},
};

type RefreshCallback = Arc<dyn Fn() + Send + Sync>;

struct RuntimeProcess {
    pid: u32,
    child: Arc<Mutex<Child>>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
}

pub struct ProcessManager {
    processes: Arc<Mutex<HashMap<String, RuntimeProcess>>>,
    states: Arc<RwLock<HashMap<String, RuntimeServiceState>>>,
    refresh: Arc<RwLock<Option<RefreshCallback>>>,
}

impl ProcessManager {
    pub fn new(states: Arc<RwLock<HashMap<String, RuntimeServiceState>>>) -> Self {
        Self {
            processes: Arc::new(Mutex::new(HashMap::new())),
            states,
            refresh: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_refresh_callback(&self, callback: RefreshCallback) {
        *self.refresh.write() = Some(callback);
    }

    pub fn start(
        &self,
        service: &ServiceConfig,
        log_path: &std::path::Path,
        command_override: Option<(&str, &[String])>,
    ) -> Result<()> {
        if self.processes.lock().contains_key(&service.id) {
            bail!("service '{}' is already running", service.id);
        }

        self.set_state(
            &service.id,
            RuntimeServiceState::new(ServiceLifecycle::Starting),
        );

        let (program, args) = match command_override {
            Some((program, args)) => (program, args),
            None => (
                service
                    .command
                    .as_deref()
                    .context("process command is missing")?,
                service.args.as_slice(),
            ),
        };

        if let Err(error) = logs::ensure_log_file(log_path) {
            self.set_failed(&service.id, error.to_string());
            return Err(error);
        }
        let mut command = Command::new(program);
        command
            .args(args)
            .envs(&service.env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(cwd) = &service.cwd {
            command.current_dir(cwd);
        }
        if service_keeps_stdin(service) {
            command.stdin(Stdio::piped());
        } else {
            command.stdin(Stdio::null());
        }

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = match command
            .spawn()
            .with_context(|| format!("failed to start service '{}'", service.name))
        {
            Ok(child) => child,
            Err(error) => {
                self.set_failed(&service.id, error.to_string());
                return Err(error);
            }
        };
        let pid = child.id();
        let stdout = child.stdout.take().context("child stdout is unavailable")?;
        let stderr = child.stderr.take().context("child stderr is unavailable")?;
        let stdin = child.stdin.take();
        let child = Arc::new(Mutex::new(child));

        self.processes.lock().insert(
            service.id.clone(),
            RuntimeProcess {
                pid,
                child: child.clone(),
                stdin: Arc::new(Mutex::new(stdin)),
            },
        );
        self.set_state(
            &service.id,
            RuntimeServiceState {
                lifecycle: ServiceLifecycle::Running,
                detail: None,
                pid: Some(pid),
                last_error: None,
            },
        );

        let stdout_state = self.states.clone();
        let stdout_refresh = self.refresh.clone();
        let stdout_id = service.id.clone();
        let stdout_status = service.status.clone();
        logs::spawn_line_logger(stdout, log_path, move |line| {
            if match_status_pattern(&stdout_state, &stdout_id, stdout_status.as_ref(), line) {
                invoke_refresh(&stdout_refresh);
            }
        })?;

        let stderr_state = self.states.clone();
        let stderr_refresh = self.refresh.clone();
        let stderr_id = service.id.clone();
        let stderr_status = service.status.clone();
        logs::spawn_line_logger(stderr, log_path, move |line| {
            if match_status_pattern(&stderr_state, &stderr_id, stderr_status.as_ref(), line) {
                invoke_refresh(&stderr_refresh);
            }
        })?;

        self.spawn_exit_watcher(service.id.clone(), pid, child);
        self.notify_refresh();
        Ok(())
    }

    pub fn write_stdin(&self, service_id: &str, input: &str) -> Result<()> {
        use std::io::Write;

        let processes = self.processes.lock();
        let process = processes
            .get(service_id)
            .with_context(|| format!("service '{service_id}' is not running"))?;
        let mut stdin = process.stdin.lock();
        let stdin = stdin
            .as_mut()
            .with_context(|| format!("service '{service_id}' does not retain stdin"))?;
        stdin
            .write_all(input.as_bytes())
            .context("failed to write process stdin")?;
        stdin.flush().context("failed to flush process stdin")
    }

    pub fn stop(&self, service: &ServiceConfig) -> Result<()> {
        let pid = self
            .processes
            .lock()
            .get(&service.id)
            .map(|process| process.pid)
            .with_context(|| format!("service '{}' is not running", service.id))?;

        self.update_lifecycle(&service.id, ServiceLifecycle::Stopping);
        let result = if service.process.kill_tree {
            kill_process_tree(pid)
        } else {
            self.processes
                .lock()
                .get(&service.id)
                .context("process disappeared while stopping")?
                .child
                .lock()
                .kill()
                .context("failed to kill child process")
        };

        if let Err(error) = &result {
            self.set_failed(&service.id, error.to_string());
        }
        result
    }

    pub fn stop_all(&self, services: &[ServiceConfig]) {
        let running_ids: Vec<String> = self.processes.lock().keys().cloned().collect();
        for service_id in running_ids {
            if let Some(service) = services.iter().find(|service| service.id == service_id) {
                let _ = self.stop(service);
            }
        }
    }

    pub fn is_running(&self, service_id: &str) -> bool {
        self.processes.lock().contains_key(service_id)
    }

    fn spawn_exit_watcher(&self, service_id: String, pid: u32, child: Arc<Mutex<Child>>) {
        let processes = self.processes.clone();
        let states = self.states.clone();
        let refresh = self.refresh.clone();
        thread::spawn(move || {
            loop {
                let exit = child.lock().try_wait();
                match exit {
                    Ok(Some(status)) => {
                        let mut process_map = processes.lock();
                        if process_map
                            .get(&service_id)
                            .is_some_and(|process| process.pid == pid)
                        {
                            process_map.remove(&service_id);
                            let current =
                                states.read().get(&service_id).map(|state| state.lifecycle);
                            let lifecycle = if status.success()
                                || matches!(
                                    current,
                                    Some(ServiceLifecycle::Stopping | ServiceLifecycle::Restarting)
                                ) {
                                ServiceLifecycle::Stopped
                            } else {
                                ServiceLifecycle::Failed
                            };
                            states.write().insert(
                                service_id.clone(),
                                RuntimeServiceState {
                                    lifecycle,
                                    detail: None,
                                    pid: None,
                                    last_error: (!status.success())
                                        .then(|| format!("process exited with {status}")),
                                },
                            );
                        }
                        invoke_refresh(&refresh);
                        break;
                    }
                    Ok(None) => thread::sleep(Duration::from_millis(250)),
                    Err(error) => {
                        states.write().insert(
                            service_id.clone(),
                            RuntimeServiceState {
                                lifecycle: ServiceLifecycle::Failed,
                                detail: None,
                                pid: Some(pid),
                                last_error: Some(format!("failed to wait for process: {error}")),
                            },
                        );
                        invoke_refresh(&refresh);
                        break;
                    }
                }
            }
        });
    }

    fn set_state(&self, service_id: &str, state: RuntimeServiceState) {
        self.states.write().insert(service_id.to_string(), state);
        self.notify_refresh();
    }

    fn set_failed(&self, service_id: &str, error: String) {
        self.states.write().insert(
            service_id.to_string(),
            RuntimeServiceState {
                lifecycle: ServiceLifecycle::Failed,
                detail: None,
                pid: None,
                last_error: Some(error),
            },
        );
        self.notify_refresh();
    }

    fn update_lifecycle(&self, service_id: &str, lifecycle: ServiceLifecycle) {
        let mut states = self.states.write();
        let state = states.entry(service_id.to_string()).or_default();
        state.lifecycle = lifecycle;
        self.notify_refresh();
    }

    fn notify_refresh(&self) {
        invoke_refresh(&self.refresh);
    }
}

fn match_status_pattern(
    states: &RwLock<HashMap<String, RuntimeServiceState>>,
    service_id: &str,
    status: Option<&StatusOptions>,
    line: &str,
) -> bool {
    let Some(status) = status else {
        return false;
    };
    for (detail, patterns) in &status.patterns {
        if patterns.iter().any(|pattern| line.contains(pattern)) {
            if let Some(state) = states.write().get_mut(service_id) {
                if state.detail.as_deref() == Some(detail) {
                    return false;
                }
                state.detail = Some(detail.clone());
                return true;
            }
            return false;
        }
    }
    false
}

fn invoke_refresh(refresh: &RwLock<Option<RefreshCallback>>) {
    if let Some(callback) = refresh.read().clone() {
        callback();
    }
}

#[cfg(windows)]
fn kill_process_tree(pid: u32) -> Result<()> {
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()
        .context("failed to execute taskkill")?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("taskkill exited with {status}"))
    }
}

#[cfg(not(windows))]
fn kill_process_tree(_pid: u32) -> Result<()> {
    Err(anyhow!(
        "process tree termination is only supported on Windows"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_without_status_patterns_does_not_request_refresh() {
        let states = RwLock::new(HashMap::from([(
            "server".to_string(),
            RuntimeServiceState::new(ServiceLifecycle::Running),
        )]));

        assert!(!match_status_pattern(
            &states,
            "server",
            None,
            "ordinary process output"
        ));
    }

    #[test]
    fn status_pattern_only_changes_state_once() {
        let states = RwLock::new(HashMap::from([(
            "metro".to_string(),
            RuntimeServiceState::new(ServiceLifecycle::Running),
        )]));
        let status = StatusOptions {
            mode: Some("output-pattern".to_string()),
            patterns: HashMap::from([("ready".to_string(), vec!["Metro waiting on".to_string()])]),
        };

        assert!(match_status_pattern(
            &states,
            "metro",
            Some(&status),
            "Metro waiting on port 8081"
        ));
        assert!(!match_status_pattern(
            &states,
            "metro",
            Some(&status),
            "Metro waiting on port 8081"
        ));
    }
}
