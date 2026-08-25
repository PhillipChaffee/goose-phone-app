//! Spawns the mock as a real process and talks to it with the real client.
//!
//! This layer exists because of the one class of bug neither side's unit
//! tests can see: a method string. `_goose/unstable/recipes/list` in the
//! client and `_goose/unstable/recipe/list` in the mock both pass every test
//! that stops at the edge of their own crate — the client's stub answers
//! whatever it is asked, the mock's handler is called directly by name — and
//! the app then shows an empty screen against a server that has the data.
//! Only sending the string over a socket to the other implementation catches
//! it, so anything that adds a `_goose/unstable/...` method gets a test here.
//!
//! The port is always 0: the OS picks, the mock prints what it got, and this
//! reads it back. Six test files each hardcoding a port would collide under
//! `cargo test`, which runs them at the same time.

// Test harness: a failing unwrap here IS the failing check, and it fires
// before any assertion in the test that called us. `expect` rather than
// `allow`: if a use goes away, so should its exception.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test harness: an unwrap or a panic is the assertion"
)]

use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Duration;

use goose_acp_client::{AcpClient, AcpEvent, ConnectConfig};
use tokio::sync::mpsc::Receiver;

const SECRET: &str = "harness-secret";

/// A running mock process. Dropping it kills the process — tests fail by
/// panicking, so anything that is not in a `Drop` does not happen on the path
/// that matters.
pub(crate) struct Server {
    child: Child,
    /// The client's event stream: `session/update` notifications, permission
    /// requests, and the final `Disconnected`. Held here rather than dropped
    /// so a test can await what a call pushed back at it.
    pub(crate) events: Receiver<AcpEvent>,
    /// The pipe the banner was read from, kept open for the process's
    /// lifetime: a child that writes to a closed stdout dies of SIGPIPE, and
    /// this one may grow more startup output later.
    _stdout: BufReader<ChildStdout>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Start the mock on an ephemeral port and connect a real [`AcpClient`] to it.
pub(crate) async fn spawn_mock() -> (Server, AcpClient) {
    spawn_mock_with(&[]).await
}

/// The same, with the fixture switches set.
///
/// They are read once at startup — that is what makes the banner able to
/// describe them — so a test that wants `MOCK_NO_SCHEDULER` or a fixture set
/// other than `full` spawns its own server rather than asking a running one to
/// change its mind. Every server here is its own process on its own port, so
/// two such tests do not see each other.
pub(crate) async fn spawn_mock_with(env: &[(&str, &str)]) -> (Server, AcpClient) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mock-goose-server"));
    command.arg("0").env("MOCK_SECRET", SECRET);
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.stdout(Stdio::piped()).spawn().unwrap();

    // The banner is written after `bind`, so having read it is proof the
    // listener is up: no polling, no sleep-and-hope.
    let stdout = child.stdout.take().unwrap();
    let read_banner = tokio::task::spawn_blocking(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let read = reader.read_line(&mut line).unwrap();
        assert!(read > 0, "mock exited before printing its address");
        (line, reader)
    });
    let (banner, stdout) = tokio::time::timeout(Duration::from_secs(10), read_banner)
        .await
        .expect("mock did not print its address within 10s")
        .unwrap();

    let base_url = banner
        .split_whitespace()
        .find(|word| word.starts_with("http://"))
        .unwrap_or_else(|| panic!("no address in mock banner: {banner}"))
        .to_string();

    let cfg = ConnectConfig {
        base_url,
        secret: SECRET.to_string(),
        fingerprint: None,
    };
    let (client, events, _info) = AcpClient::connect(&cfg).await.unwrap();

    (
        Server {
            child,
            events,
            _stdout: stdout,
        },
        client,
    )
}
