mod config;
mod logs;
mod paths;
mod process_manager;
mod react_native;
mod service;
mod terminal;
mod tray;
mod windows_service;

use std::sync::Arc;

use anyhow::Context;
use tauri::{Manager, RunEvent, WindowEvent};

pub fn run() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let loaded = config::load_or_create().context("failed to initialize DevDock config")?;
            let registry = Arc::new(service::ServiceRegistry::new(loaded));
            app.manage(registry.clone());

            tray::create(app.handle(), registry.clone())?;
            let app_handle = app.handle().clone();
            let refresh_registry = registry.clone();
            registry.set_refresh_callback(Arc::new(move || {
                tray::request_refresh(&app_handle, refresh_registry.clone());
            }));
            registry.start_launch_services();
            Ok(())
        })
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(error) => {
            eprintln!("failed to build DevDock: {error:#}");
            return;
        }
    };

    app.run(|app, event| match event {
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } if label == "main" => {
            api.prevent_close();
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
        }
        _ => {}
    });
}
