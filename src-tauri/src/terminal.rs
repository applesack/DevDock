use std::{path::Path, process::Command};

use anyhow::{Context, Result, bail};

use crate::config::TerminalConfig;

pub fn open_log_in_terminal(
    terminal: &TerminalConfig,
    service_name: &str,
    log_file: &Path,
) -> Result<()> {
    let escaped_path = powershell_single_quote(&log_file.to_string_lossy());
    let command = terminal.tail_command.replace("{logFile}", &escaped_path);

    Command::new(&terminal.program)
        .args([
            "new-tab",
            "--title",
            service_name,
            &terminal.shell,
            "-NoExit",
            "-Command",
            &command,
        ])
        .spawn()
        .with_context(|| format!("failed to launch {}", terminal.program))?;
    Ok(())
}

pub fn open_directory(path: &Path) -> Result<()> {
    if !path.exists() {
        bail!("directory does not exist: {}", path.display());
    }
    open::that(path).with_context(|| format!("failed to open directory {}", path.display()))
}

fn powershell_single_quote(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_powershell_single_quotes() {
        assert_eq!(
            powershell_single_quote("C:/Bob's/log.txt"),
            "C:/Bob''s/log.txt"
        );
    }
}
