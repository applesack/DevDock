use std::{path::Path, process::Command};

use anyhow::{Context, Result, bail};

use crate::config::TerminalConfig;

const LEGACY_TAIL_COMMAND: &str = "Get-Content -Path '{logFile}' -Wait";
const DEFAULT_TAIL_COMMAND: &str = "Get-Content -Path '{logFile}' -Tail 300 -Wait";

pub fn open_log_in_terminal(
    terminal: &TerminalConfig,
    service_name: &str,
    log_file: &Path,
) -> Result<()> {
    let command = build_tail_command(&terminal.tail_command, log_file);

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

fn build_tail_command(template: &str, log_file: &Path) -> String {
    let template = if template == LEGACY_TAIL_COMMAND {
        DEFAULT_TAIL_COMMAND
    } else {
        template
    };
    let escaped_path = powershell_single_quote(&log_file.to_string_lossy());
    template.replace("{logFile}", &escaped_path)
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

    #[test]
    fn legacy_tail_command_only_loads_last_300_lines() {
        assert_eq!(
            build_tail_command(LEGACY_TAIL_COMMAND, Path::new("C:/logs/api.log")),
            "Get-Content -Path 'C:/logs/api.log' -Tail 300 -Wait"
        );
    }

    #[test]
    fn custom_tail_command_is_preserved() {
        assert_eq!(
            build_tail_command("custom-tail '{logFile}'", Path::new("C:/logs/api.log")),
            "custom-tail 'C:/logs/api.log'"
        );
    }
}
