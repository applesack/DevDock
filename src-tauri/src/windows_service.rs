use std::process::{Command, Output};

use anyhow::{Context, Result, bail};

use crate::service::ServiceLifecycle;

pub fn query(service_name: &str) -> Result<ServiceLifecycle> {
    let output = run_sc(["query", service_name])?;
    if !output.status.success() {
        bail!("sc.exe query failed: {}", output_text(&output));
    }
    let text = output_text(&output).to_ascii_uppercase();
    if text.contains("RUNNING") {
        Ok(ServiceLifecycle::Running)
    } else if text.contains("START_PENDING") {
        Ok(ServiceLifecycle::Starting)
    } else if text.contains("STOP_PENDING") {
        Ok(ServiceLifecycle::Stopping)
    } else if text.contains("STOPPED") {
        Ok(ServiceLifecycle::Stopped)
    } else {
        Ok(ServiceLifecycle::Unknown)
    }
}

pub fn start(service_name: &str) -> Result<()> {
    let output = run_sc(["start", service_name])?;
    if !output.status.success() {
        bail!("sc.exe start failed: {}", output_text(&output));
    }
    Ok(())
}

pub fn stop(service_name: &str) -> Result<()> {
    let output = run_sc(["stop", service_name])?;
    if !output.status.success() {
        bail!("sc.exe stop failed: {}", output_text(&output));
    }
    Ok(())
}

fn run_sc<const N: usize>(args: [&str; N]) -> Result<Output> {
    Command::new("sc.exe")
        .args(args)
        .output()
        .context("failed to execute sc.exe")
}

fn output_text(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!("{} {}", stdout.trim(), stderr.trim())
        .trim()
        .to_string()
}
