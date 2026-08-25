#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Dioxus's rsx!/#[component] macros expand to fully-qualified paths, so
// unused_qualifications fires ~50 times on code no one here can edit. Scoped
// to this crate: the library crates have no macro expansion to excuse and
// keep the lint enforced (that is how the one real occurrence, a qualified
// CryptoProvider::get_default() in goose-acp-client, was found).
#![allow(unused_qualifications, reason = "Dioxus macro expansion")]

// One blank line between every declaration, so five branches each adding a
// module of their own land in five separate hunks instead of one contested
// list.

mod app;

mod attach;

mod code;

mod css;

mod diff;

#[cfg(debug_assertions)]
mod domdump;

mod extensions;

mod external;

mod icons;

mod markdown;

mod nav;

mod skills;

mod state;

mod viewport;

mod views;

fn main() {
    // Persisted settings live in the app-private data dir (Android:
    // getFilesDir() via JNI, iOS/desktop: the platform data dir).
    dioxus_sdk_storage::set_dir!();
    goose_acp_client::ensure_crypto_provider();
    dioxus::launch(app::App);
}
