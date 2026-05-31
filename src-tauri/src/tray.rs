use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

use crate::commands::toggle_recording;

pub const TRAY_ID: &str = "ears-tray";

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let start_item = MenuItem::with_id(app, "start", "Start Recording", true, None::<&str>)?;
    let open_item = MenuItem::with_id(app, "open", "Settings", true, None::<&str>)?;
    let last_result_item = MenuItem::with_id(
        app,
        "last_result",
        "No transcription yet",
        false,
        None::<&str>,
    )?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    {
        let state = app.state::<crate::state::AppState>();
        *state.last_result_menu_item.lock().unwrap() = Some(last_result_item.clone());
        *state.start_stop_menu_item.lock().unwrap() = Some(start_item.clone());
    }

    let menu = Menu::with_items(app, &[&start_item, &open_item, &last_result_item, &sep, &quit_item])?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(include_bytes!("../icons-tray/idle.png"))?)
        .title("ears")
        .tooltip("ears - click to record")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = event
            {
                let app = tray.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = toggle_recording(app).await {
                        eprintln!("toggle recording error: {e}");
                    }
                });
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            "start" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = toggle_recording(app).await {
                        eprintln!("toggle recording error: {e}");
                    }
                });
            }
            "open" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}

pub fn set_tray_recording(app: &AppHandle, recording: bool) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let icon_bytes: &[u8] = if recording {
            include_bytes!("../icons-tray/recording.png")
        } else {
            include_bytes!("../icons-tray/idle.png")
        };
        if let Ok(icon) = Image::from_bytes(icon_bytes) {
            let _ = tray.set_icon(Some(icon));
        }
        let tooltip = if recording {
            "ears - recording (click to stop)"
        } else {
            "ears - click to record"
        };
        let _ = tray.set_tooltip(Some(tooltip));
    }

    let state = app.state::<crate::state::AppState>();
    let guard = state.start_stop_menu_item.lock().unwrap();
    if let Some(item) = &*guard {
        let label = if recording { "Stop Recording" } else { "Start Recording" };
        let _ = item.set_text(label);
    }
}

pub fn set_tray_last_result(app: &AppHandle, text: &str) {
    let state = app.state::<crate::state::AppState>();
    let guard = state.last_result_menu_item.lock().unwrap();
    if let Some(item) = &*guard {
        let preview: String = text.chars().take(50).collect();
        let label = if text.chars().count() > 50 { format!("{preview}...") } else { preview };
        let _ = item.set_text(label);
        let _ = item.set_enabled(true);
    }
}
