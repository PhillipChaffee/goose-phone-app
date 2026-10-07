//! PROTOTYPE — throwaway. The on-screen half of the walking skeleton
//! (wayfinder ticket 10): launches the five-beat runner from
//! `acp-walking-skeleton` against whatever `SKELETON_URL` names, and prints
//! every beat arrival as it happens. Gated out of any shipped binary by
//! `cfg(debug_assertions)`; rendered only when the app is launched with
//! `GOOSE_SKELETON=1`. Styling is inline on purpose — this screen must not
//! register any class the design-system gates would have to audit.

use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;

/// One launch variable with a prototype fallback: the tailnet URL the
/// connectivity decision (07) prescribed, the secret `serve-real.sh`-style
/// tests run with, and the server-side working directory.
fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// The whole walking skeleton in one screen: every beat line, in arrival
/// order, with its elapsed time — which is what makes "deltas stream" visible
/// without a stopwatch.
#[component]
pub fn SkeletonView() -> Element {
    let mut lines: Signal<Vec<String>> = use_signal(Vec::new);

    use_effect(move || {
        let cfg = acp_walking_skeleton::SkeletonConfig {
            base_url: env_or("SKELETON_URL", "https://macbook-pro-2.tail5ac550.ts.net:9443"),
            secret: env_or("SKELETON_SECRET", "skeleton-secret"),
            cwd: env_or(
                "SKELETON_CWD",
                "/Users/phillipchaffee/git/goose-phone-app-scratch",
            ),
            prompt: env_or(
                "SKELETON_PROMPT",
                "Create a file named skeleton-note.txt in the working directory containing the words hello skeleton, then confirm what you wrote.",
            ),
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<acp_walking_skeleton::Beat>();
        // The runner wants a tokio reactor of its own (tokio::time::sleep,
        // reqwest IO) and its future is not Dioxus-runtime-shaped, so it gets
        // a dedicated thread; the beat channel is the bridge back to signals.
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build();
            if let Ok(runtime) = runtime {
                runtime.block_on(acp_walking_skeleton::run(cfg, tx));
            }
        });
        spawn_forever(async move {
            while let Some(beat) = rx.recv().await {
                lines.push(format!(
                    "[+{:>6}ms] beat{}: {}",
                    beat.at_ms, beat.beat, beat.text
                ));
            }
        });
        let keystore = match acp_walking_skeleton::keystore_probe() {
            Ok(text) => format!("keystore: {text}"),
            Err(text) => format!("keystore FAILED: {text}"),
        };
        lines.push(keystore);
    });

    rsx! {
        div {
            style: "background:#111;color:#ddd;font-family:ui-monospace,Menlo,monospace;font-size:11px;line-height:1.5;padding:12px;padding-top:max(12px,env(safe-area-inset-top));height:100vh;overflow-y:auto;-webkit-user-select:text;user-select:text;word-break:break-word;white-space:pre-wrap",
            for line in lines.read().iter() {
                div { "{line}" }
            }
        }
    }
}
