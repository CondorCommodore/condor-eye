mod config;
mod compare;

use std::sync::Mutex;
use tauri::Manager;

pub struct AppState {
    pub config: Mutex<config::AppConfig>,
}

fn main() {
    let app_config = config::AppConfig::from_env();

    tauri::Builder::default()
        .manage(AppState {
            config: Mutex::new(app_config),
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
