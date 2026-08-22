#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod markdown;
mod state;
mod views;

fn main() {
    // Persisted settings live in the app-private data dir (Android:
    // getFilesDir() via JNI, iOS/desktop: the platform data dir).
    dioxus_sdk_storage::set_dir!();
    goose_acp_client::ensure_crypto_provider();
    dioxus::launch(app::App);
}
