//! PROTOTYPE preflight — run the five beats from the command line against a
//! `goose serve` (or the mock) to settle the seam before it rides a phone.
use acp_walking_skeleton::SkeletonConfig;

#[tokio::main]
async fn main() {
    let url = std::env::var("SKELETON_URL").unwrap_or_else(|_| "http://127.0.0.1:3286".into());
    let secret = std::env::var("SKELETON_SECRET").unwrap_or_else(|_| "skeleton-secret".into());
    let cwd = std::env::var("GOOSE_DEV_WORKING_DIR")
        .unwrap_or_else(|_| std::env::temp_dir().display().to_string());
    let prompt = std::env::args().nth(1).unwrap_or_else(|| {
        "Create a file named skeleton-note.txt in the working directory containing the words hello skeleton, then confirm what you wrote."
            .into()
    });

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<acp_walking_skeleton::Beat>();
    let cfg = SkeletonConfig {
        base_url: url,
        secret,
        cwd,
        prompt,
    };

    tokio::spawn(acp_walking_skeleton::run(cfg, tx));

    while let Some(line) = rx.recv().await {
        println!("[+{:>6}ms] beat{}: {}", line.at_ms, line.beat, line.text);
        if line.text.starts_with("SKELETON") {
            break;
        }
    }
    match acp_walking_skeleton::keystore_probe() {
        Ok(msg) => println!("keystore probe: {msg}"),
        Err(e) => println!("keystore probe FAILED: {e}"),
    }
}
