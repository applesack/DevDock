use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::Path,
    thread,
};

use anyhow::{Context, Result};

pub fn ensure_log_file(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create log directory {}", parent.display()))?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to create log file {}", path.display()))?;
    Ok(())
}

pub fn open_log(path: &Path) -> Result<()> {
    ensure_log_file(path)?;
    open::that(path).with_context(|| format!("failed to open log file {}", path.display()))
}

pub fn spawn_line_logger<R, F>(reader: R, path: &Path, on_line: F) -> Result<()>
where
    R: std::io::Read + Send + 'static,
    F: Fn(&str) + Send + 'static,
{
    ensure_log_file(path)?;
    let file = open_append(path)?;
    thread::spawn(move || {
        let reader = BufReader::new(reader);
        let mut file = file;
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    let _ = writeln!(file, "{line}");
                    let _ = file.flush();
                    on_line(&line);
                }
                Err(error) => {
                    let _ = writeln!(file, "[DevDock] failed to read process output: {error}");
                    break;
                }
            }
        }
    });
    Ok(())
}

pub fn write_app_error(log_dir: &Path, message: &str) {
    let path = log_dir.join("devdock.log");
    if let Ok(mut file) = open_append(&path) {
        let _ = writeln!(file, "[ERROR] {message}");
    }
}

fn open_append(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create log directory {}", parent.display()))?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open log file {}", path.display()))
}
