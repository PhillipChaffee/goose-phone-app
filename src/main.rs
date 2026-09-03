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

mod ask_journal;

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

#[cfg(test)]
mod selfscan;

// Stands a JSON-RPC server up on a loopback port and connects a real
// `AcpClient` to it, so a feature module's with-a-client half can be driven at
// all. `cfg(test)` like `selfscan` above, so none of it is in any binary.
#[cfg(test)]
mod serverkit;

mod shell;

mod skills;

mod state;

// Mounts a view with a real `AppCtx` under it and hands back the markup.
// `cfg(test)` like `selfscan` above, so none of it is in any binary.
#[cfg(test)]
mod testkit;

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
/// One gate, and it is the target: `Cargo.toml` gives `dioxus` its `desktop`
/// feature only under this same predicate, so `dioxus::desktop` does not
/// exist on a phone and this arm could not compile there even if the `cfg`
/// were forgotten.
///
/// The numbers, and where they come from, are in `crate::shell` beside the
/// breakpoints they have to agree with; a test there checks that they do.
/// The opening size clears the three-column breakpoint comfortably, so a first
/// launch shows the layout the app was designed as rather than a fallback.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn launch() {
    use dioxus::desktop::{Config, LogicalSize, WindowBuilder};

    let (min_w, min_h) = shell::MIN_INNER;
    let window = WindowBuilder::new()
        .with_title("Goose")
        // 1440 IS THE MOCKUPS' OWN WIDTH, and it is arithmetic rather than
        // deference. The sidebar and the inspector are 268 + 344 = 612 of
        // chrome. At the 1180 this opened at, that leaves 568 for the content
        // column — under the 40rem (640px) `--measure` `.pane-main` is built
        // around, so every screen would render at its cap-collapsed width and
        // the inspector would have shipped having quietly narrowed the thing
        // it comments on. At 1440 the content column is 828: the 640 measure
        // plus 94 of gutter each side, which is what the mockups draw.
        //
        // 860 AND NOT THE MOCKUPS' 900. A 1440x900 display has about 875 of
        // usable height once the menu bar is out, so a 900-tall window opens
        // taller than the screen it is on. 860 clears it and is still 40 more
        // than before. `with_min_inner_size` is untouched — see `MIN_INNER`.
        .with_inner_size(LogicalSize::new(1440.0, 860.0))
        .with_min_inner_size(LogicalSize::new(min_w, min_h));
    let window = integrate_titlebar(window);
    dioxus::LaunchBuilder::desktop()
        .with_cfg(Config::new().with_window(window))
        .launch(app::App);
}

/// Put the window chrome INSIDE the app instead of above it.
///
/// A stock macOS window puts a 28pt opaque bar over the content, in the
/// system's grey rather than the app's, with the app's own surface starting
/// underneath it — so the first thing on screen is a strip that belongs to
/// nothing. goose's own desktop app does not have one: `titleBarStyle:
/// 'hidden'` with `trafficLightPosition: { x: 20, y: 16 }`
/// (`ui/desktop/src/main.ts:1285-1286`), which is the same three flags below
/// plus an inset.
///
/// Three flags and they are not interchangeable. `fullsize_content_view` is
/// what extends the web view up under the titlebar; `titlebar_transparent`
/// stops the bar painting its own material over it; `title_hidden` removes
/// the word "Goose", which would otherwise float over the app's content in a
/// font the app does not own. NOT `titlebar_hidden`, which takes the traffic
/// lights with it.
///
/// NO INSET, and that is a finding rather than a preference. tao exposes
/// `with_traffic_light_inset`, goose sets the Electron equivalent to (20, 16),
/// and under wry it does nothing at all: tao applies the inset from the
/// content view's `drawRect:` (`tao-0.34.8/src/platform_impl/macos/view.rs:346`)
/// and wry's `WKWebView` covers that view, so it never draws and the hook
/// never runs. Measured on a real window at two different insets — (20, 18)
/// and (31, 25) — the lights did not move: both put the close button at
/// logical (9, 9). So the strip in `assets/desktop/` is sized to where
/// macOS actually puts them, and `--traffic-w` records where they end.
///
/// `src/shell/desktop/mod.rs` renders the region that drags the window: with the
/// bar gone, `AppKit` no longer has a strip of its own to drag by, and a
/// window you cannot move is a worse bug than a grey bar.
///
/// macOS only. Two whole functions rather than one with a `cfg` block inside,
/// because a `cfg`'d block that has to `return` so a `cfg`'d tail expression
/// can follow it is a shape clippy's `needless_return` correctly rejects: on
/// the platform being compiled, only one of the two ever exists.
#[cfg(target_os = "macos")]
fn integrate_titlebar(window: dioxus::desktop::WindowBuilder) -> dioxus::desktop::WindowBuilder {
    use dioxus::desktop::tao::platform::macos::WindowBuilderExtMacOS;

    window
        .with_fullsize_content_view(true)
        .with_titlebar_transparent(true)
        .with_title_hidden(true)
}

/// Everywhere else the window keeps its native frame: a Windows or Linux
/// window with no titlebar and no replacement for it is a window with no
/// close button.
#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "android")))]
const fn integrate_titlebar(
    window: dioxus::desktop::WindowBuilder,
) -> dioxus::desktop::WindowBuilder {
    window
}

/// The phone's launch: `dioxus::launch` picks the mobile renderer, which is
/// the same crate the arm above builds a window with, reached through the
/// `mobile` feature `Cargo.toml` gives it on exactly these two targets.
#[cfg(any(target_os = "ios", target_os = "android"))]
fn launch() {
    dioxus::launch(app::App);
}
