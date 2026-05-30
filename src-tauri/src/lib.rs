mod audio;
mod commands;
mod models;
mod state;
mod transcribe;
mod tray;

use commands::*;
use state::AppState;
use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.get(1).map(|s| s.as_str()) == Some("toggle") {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = commands::toggle_recording(app).await {
                        eprintln!("toggle error: {e}");
                    }
                });
            }
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_notification::init())
        .manage(AppState::new())
        .setup(|app| {
            let handle = app.handle().clone();

            // Load persisted settings before tray setup
            let app_state = app.state::<AppState>();
            commands::load_settings(&handle, &app_state);

            tray::setup_tray(&handle)?;

            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();

                let win = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win.hide();
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            list_models,
            download_model,
            delete_model,
            get_status,
            get_history,
            remove_history_item,
            cmd_toggle_recording,
        ])
        .run(tauri::generate_context!())
        .expect("error running ears");
}
