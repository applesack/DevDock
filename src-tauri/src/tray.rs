use std::sync::Arc;

use anyhow::{Context, Result};
use tauri::{
    AppHandle, Manager, Wry,
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use crate::{
    config::{ServiceConfig, ServiceType},
    logs,
    service::{ServiceLifecycle, ServiceRegistry, action_is_available},
    terminal,
};

pub const TRAY_ID: &str = "devdock-main";
const REACT_NATIVE_RELOAD_ACTION_ID: &str = "reload";
const REACT_NATIVE_OPEN_DEV_MENU_ACTION_ID: &str = "open-dev-menu";

pub fn create(app: &AppHandle, registry: Arc<ServiceRegistry>) -> Result<()> {
    let menu = build_menu(app, &registry)?;
    let event_registry = registry.clone();
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(create_icon())
        .tooltip("DevDock")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| {
            handle_menu_event(app.clone(), event.id().as_ref(), event_registry.clone());
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
                && let Some(window) = tray.app_handle().get_webview_window("main")
            {
                let _ = window.show();
                let _ = window.set_focus();
            }
        })
        .build(app)?;
    Ok(())
}

pub fn refresh(app: &AppHandle, registry: &ServiceRegistry) -> Result<()> {
    let menu = build_menu(app, registry)?;
    app.tray_by_id(TRAY_ID)
        .context("DevDock tray icon is unavailable")?
        .set_menu(Some(menu))?;
    Ok(())
}

fn build_menu(app: &AppHandle, registry: &ServiceRegistry) -> Result<Menu<Wry>> {
    let loaded = registry.config();
    let menu = Menu::new(app)?;

    for service in &loaded.config.services {
        menu.append(&build_service_menu(app, registry, service)?)?;
    }

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    let settings_menu = Submenu::with_id(app, "app:settings", "Settings", true)?;
    settings_menu.append(&MenuItem::with_id(
        app,
        "app:reload-config",
        "Reload Config",
        true,
        None::<&str>,
    )?)?;
    settings_menu.append(&MenuItem::with_id(
        app,
        "app:open-config",
        "Open Config File",
        true,
        None::<&str>,
    )?)?;
    settings_menu.append(&MenuItem::with_id(
        app,
        "app:open-logs",
        "Open Logs Directory",
        true,
        None::<&str>,
    )?)?;
    menu.append(&settings_menu)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "app:quit",
        "Quit",
        true,
        None::<&str>,
    )?)?;

    Ok(menu)
}

fn build_service_menu(
    app: &AppHandle,
    registry: &ServiceRegistry,
    service: &ServiceConfig,
) -> Result<Submenu<Wry>> {
    let state = registry.get_service_state(&service.id);
    let detail = state
        .detail
        .as_deref()
        .map(title_case)
        .map(|detail| format!(" - {detail}"))
        .unwrap_or_default();
    let title = format!("{}{}", service.name, detail);
    let submenu = Submenu::with_id_and_icon(
        app,
        format!("service-menu:{}", service.id),
        title,
        true,
        Some(create_status_icon(state.lifecycle)),
    )?;

    match state.lifecycle {
        ServiceLifecycle::Running | ServiceLifecycle::Starting | ServiceLifecycle::Restarting => {
            submenu.append(&service_item(app, "stop", service, "Stop")?)?;
            submenu.append(&service_item(app, "restart", service, "Restart")?)?;
        }
        ServiceLifecycle::Failed => {
            submenu.append(&service_item(app, "start", service, "Start")?)?;
            submenu.append(&service_item(app, "restart", service, "Restart")?)?;
        }
        _ => {
            submenu.append(&service_item(app, "start", service, "Start")?)?;
        }
    }

    submenu.append(&PredefinedMenuItem::separator(app)?)?;
    submenu.append(&service_item(app, "open-log", service, "Open Log")?)?;
    submenu.append(&service_item(
        app,
        "open-log-terminal",
        service,
        "Open Log in Terminal",
    )?)?;

    if service.service_type == ServiceType::ReactNative
        && state.lifecycle == ServiceLifecycle::Running
    {
        submenu.append(&PredefinedMenuItem::separator(app)?)?;
        submenu.append(&react_native_item(
            app,
            REACT_NATIVE_RELOAD_ACTION_ID,
            service,
            "Reload app",
        )?)?;
        submenu.append(&react_native_item(
            app,
            REACT_NATIVE_OPEN_DEV_MENU_ACTION_ID,
            service,
            "Open dev menu",
        )?)?;
    }

    let available_actions: Vec<_> = service
        .actions
        .iter()
        .filter(|action| {
            service.service_type != ServiceType::ReactNative
                || !matches!(
                    action.id.as_str(),
                    REACT_NATIVE_RELOAD_ACTION_ID | REACT_NATIVE_OPEN_DEV_MENU_ACTION_ID
                )
        })
        .filter(|action| action_is_available(action.when, state.lifecycle))
        .collect();
    if !available_actions.is_empty() {
        submenu.append(&PredefinedMenuItem::separator(app)?)?;
        for action in available_actions {
            submenu.append(&MenuItem::with_id(
                app,
                format!("action:{}:{}", service.id, action.id),
                &action.label,
                true,
                None::<&str>,
            )?)?;
        }
    }

    Ok(submenu)
}

fn service_item(
    app: &AppHandle,
    operation: &str,
    service: &ServiceConfig,
    label: &str,
) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(
        app,
        format!("service:{operation}:{}", service.id),
        label,
        true,
        None::<&str>,
    )
}

fn react_native_item(
    app: &AppHandle,
    operation: &str,
    service: &ServiceConfig,
    label: &str,
) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(
        app,
        format!("react-native:{operation}:{}", service.id),
        label,
        true,
        None::<&str>,
    )
}

fn handle_menu_event(app: AppHandle, id: &str, registry: Arc<ServiceRegistry>) {
    if id == "app:quit" {
        quit(app, registry);
        return;
    }

    let id = id.to_string();
    std::thread::spawn(move || {
        let result = match id.as_str() {
            "app:reload-config" => reload_config(&registry),
            "app:open-config" => {
                let path = registry.config().config_path;
                open::that(&path)
                    .with_context(|| format!("failed to open config file {}", path.display()))
            }
            "app:open-logs" => terminal::open_directory(&registry.config().log_dir),
            _ => dispatch_service_event(&registry, &id),
        };

        if let Err(error) = result {
            let loaded = registry.config();
            logs::write_app_error(&loaded.log_dir, &error.to_string());
            eprintln!("DevDock: {error:#}");
        }
        request_refresh(&app, registry);
    });
}

fn quit(app: AppHandle, registry: Arc<ServiceRegistry>) {
    std::thread::spawn(move || {
        registry.stop_all();
        let app_for_exit = app.clone();
        if let Err(error) = app.run_on_main_thread(move || app_for_exit.exit(0)) {
            let loaded = registry.config();
            logs::write_app_error(
                &loaded.log_dir,
                &format!("failed to schedule DevDock exit: {error}"),
            );
        }
    });
}

fn dispatch_service_event(registry: &ServiceRegistry, id: &str) -> Result<()> {
    if let Some(rest) = id.strip_prefix("service:") {
        let (operation, service_id) = rest.split_once(':').context("invalid service menu event")?;
        return match operation {
            "start" => registry.start_service(service_id),
            "stop" => registry.stop_service(service_id),
            "restart" => registry.restart_service(service_id),
            "open-log" => registry.open_log(service_id),
            "open-log-terminal" => registry.open_log_in_terminal(service_id),
            _ => anyhow::bail!("unknown service operation '{operation}'"),
        };
    }

    if let Some(rest) = id.strip_prefix("action:") {
        let (service_id, action_id) = rest.split_once(':').context("invalid action menu event")?;
        return registry.run_action(service_id, action_id);
    }

    if let Some(rest) = id.strip_prefix("react-native:") {
        let (operation, service_id) = rest
            .split_once(':')
            .context("invalid react-native menu event")?;
        let input = match operation {
            REACT_NATIVE_RELOAD_ACTION_ID => "r",
            REACT_NATIVE_OPEN_DEV_MENU_ACTION_ID => "d",
            _ => anyhow::bail!("unknown react-native operation '{operation}'"),
        };
        return registry.run_react_native_command(service_id, input);
    }

    anyhow::bail!("unknown tray menu event '{id}'")
}

fn reload_config(registry: &ServiceRegistry) -> Result<()> {
    let path = registry.config().config_path;
    let loaded = crate::config::load_from_path(&path)?;
    registry.reload_config(loaded);
    Ok(())
}

pub fn request_refresh(app: &AppHandle, registry: Arc<ServiceRegistry>) {
    let app_for_task = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Err(error) = refresh(&app_for_task, &registry) {
            let loaded = registry.config();
            logs::write_app_error(&loaded.log_dir, &error.to_string());
        }
    });
}

fn title_case(value: &str) -> String {
    let mut characters = value.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

fn create_icon() -> Image<'static> {
    const SIZE: usize = 32;
    let mut rgba = vec![0_u8; SIZE * SIZE * 4];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let offset = (y * SIZE + x) * 4;
            let rounded_corner = !(4..=27).contains(&x) && !(4..=27).contains(&y);
            let is_d = (x == 9 && (8..24).contains(&y))
                || ((8..22).contains(&x) && (y == 8 || y == 23))
                || (x == 22 && (10..22).contains(&y));
            let color = if is_d {
                [255, 255, 255, 255]
            } else if rounded_corner {
                [0, 0, 0, 0]
            } else {
                [37, 99, 235, 255]
            };
            rgba[offset..offset + 4].copy_from_slice(&color);
        }
    }
    Image::new_owned(rgba, SIZE as u32, SIZE as u32)
}

fn create_status_icon(lifecycle: ServiceLifecycle) -> Image<'static> {
    const SIZE: usize = 16;
    const CENTER: i32 = 7;
    const OUTER_RADIUS_SQUARED: i32 = 25;
    const INNER_RADIUS_SQUARED: i32 = 16;

    let color = status_color(lifecycle);
    let mut rgba = vec![0_u8; SIZE * SIZE * 4];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as i32 - CENTER;
            let dy = y as i32 - CENTER;
            let distance = dx * dx + dy * dy;
            if distance <= OUTER_RADIUS_SQUARED {
                let offset = (y * SIZE + x) * 4;
                let pixel = if distance > INNER_RADIUS_SQUARED {
                    [148, 163, 184, 255]
                } else {
                    color
                };
                rgba[offset..offset + 4].copy_from_slice(&pixel);
            }
        }
    }
    Image::new_owned(rgba, SIZE as u32, SIZE as u32)
}

fn status_color(lifecycle: ServiceLifecycle) -> [u8; 4] {
    match lifecycle {
        ServiceLifecycle::Running | ServiceLifecycle::Starting | ServiceLifecycle::Restarting => {
            [34, 197, 94, 255]
        }
        ServiceLifecycle::Failed => [239, 68, 68, 255],
        ServiceLifecycle::Stopped | ServiceLifecycle::Stopping | ServiceLifecycle::Unknown => {
            [255, 255, 255, 255]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_colors_follow_menu_semantics() {
        assert_eq!(status_color(ServiceLifecycle::Running), [34, 197, 94, 255]);
        assert_eq!(
            status_color(ServiceLifecycle::Stopped),
            [255, 255, 255, 255]
        );
        assert_eq!(status_color(ServiceLifecycle::Failed), [239, 68, 68, 255]);
    }
}
