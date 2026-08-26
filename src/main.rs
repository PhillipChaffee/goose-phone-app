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

mod cron;

mod css;

mod diff;

#[cfg(debug_assertions)]
mod domdump;

mod extensions;

mod external;

mod icons;

mod markdown;

mod nav;

mod recipes;

mod scheduler;

mod shell;

mod skills;

mod state;

mod viewport;

mod views;

fn main() {
    // Persisted settings live in the app-private data dir (Android:
    // getFilesDir() via JNI, iOS/desktop: the platform data dir).
    dioxus_sdk_storage::set_dir!();
    goose_acp_client::ensure_crypto_provider();
    launch();
}

/// Open the window the desktop shell was designed for, and refuse to let it be
/// dragged smaller than the shell can draw.
///
/// Both halves of the `cfg` are load-bearing. `feature = "desktop"` because
/// the `dioxus::desktop` module only exists under it, and
/// `not(ios/android)` because `cargo check --target aarch64-apple-ios` runs
/// with DEFAULT features — which is `desktop` — and must not try to build a
/// tao window for a phone.
///
/// The numbers, and where they come from, are in `crate::shell` beside the
/// breakpoints they have to agree with; a test there checks that they do.
/// The opening size clears the three-column breakpoint comfortably, so a first
/// launch shows the layout the app was designed as rather than a fallback.
#[cfg(all(
    feature = "desktop",
    not(any(target_os = "ios", target_os = "android"))
))]
fn launch() {
    use dioxus::desktop::{Config, LogicalSize, WindowBuilder};

    let (min_w, min_h) = shell::MIN_INNER;
    let window = WindowBuilder::new()
        .with_title("Goose")
        .with_inner_size(LogicalSize::new(1180.0, 820.0))
        .with_min_inner_size(LogicalSize::new(min_w, min_h));
    dioxus::LaunchBuilder::desktop()
        .with_cfg(Config::new().with_window(window))
        .launch(app::App);
}

#[cfg(not(all(
    feature = "desktop",
    not(any(target_os = "ios", target_os = "android"))
)))]
fn launch() {
    dioxus::launch(app::App);
}
