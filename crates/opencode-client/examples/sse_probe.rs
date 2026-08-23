//! Diagnostic: attach to one chat's SSE stream and print raw event arrivals
//! with timing. `CODE_BASE_URL`, `CODE_PASSWORD`, `CODE_CHAT_ID` env vars.

// Diagnostic example: a missing env var should stop the probe on the spot,
// and stdout is its entire output. Both are denied for shipped code.
// `expect` rather than `allow`: if a use goes away, so should its exception.
#![expect(
    clippy::unwrap_used,
    clippy::print_stdout,
    reason = "test/example harness: assertions and progress output are the point"
)]

use opencode_client::{CodeClient, CodeConfig};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let client = CodeClient::new(&CodeConfig {
        base_url: std::env::var("CODE_BASE_URL").unwrap(),
        password: std::env::var("CODE_PASSWORD").unwrap(),
    })
    .unwrap();
    let chat_id = std::env::var("CODE_CHAT_ID").unwrap();
    let start = std::time::Instant::now();
    let mut events = client.events(&chat_id);
    println!(
        "[{:>6.2}s] attached, waiting…",
        start.elapsed().as_secs_f32()
    );
    for _ in 0..5 {
        match tokio::time::timeout(std::time::Duration::from_secs(20), events.recv()).await {
            Ok(Some(e)) => println!("[{:>6.2}s] {:?}", start.elapsed().as_secs_f32(), e),
            Ok(None) => {
                println!("[{:>6.2}s] channel closed", start.elapsed().as_secs_f32());
                break;
            }
            Err(_) => {
                println!("[{:>6.2}s] 20s: nothing", start.elapsed().as_secs_f32());
                break;
            }
        }
    }
}
