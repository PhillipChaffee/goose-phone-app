//! Handing a link to the system browser.
//!
//! A pull request lives on GitHub, and GitHub in a `WKWebView` with no
//! address bar, no tabs and no sign-in is not GitHub. So the one thing this
//! app does with a `url` is give it away.
//!
//! There is no Rust route to `UIApplication.openURL` here — the app links no
//! platform crate and Dioxus exposes none — so this goes through the web view
//! the same way the gesture code in `viewport.rs` does. `window.open` with a
//! target is what `WKWebView` turns into a "this wants a new window" request,
//! which the host answers by handing the URL to the OS. The anchor after it is
//! the fallback for a build where `window.open` returns nothing: a click on a
//! `target="_blank"` link raises the same request by a different door.

use dioxus::document;

/// Whether a URL is one this app will hand to the browser.
///
/// The scheme check is not decoration. These URLs come off the wire, and
/// `window.open("javascript:…")` does not leave the app at all — it runs, in
/// the web view that is holding the session. Only http and https travel.
pub(crate) fn is_web_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// Open `url` outside the app, if it is the sort of thing that can be.
pub(crate) fn open_external(url: &str) {
    let url = url.trim();
    if !is_web_url(url) {
        return;
    }
    // Serialised as JSON rather than pasted between quotes: a URL with an
    // apostrophe in it would otherwise close the string and the rest of it
    // would be script.
    let Ok(literal) = serde_json::to_string(url) else {
        return;
    };
    document::eval(&format!(
        "(() => {{ const u = {literal}; \
           if (window.open(u, '_blank', 'noopener')) return; \
           const a = document.createElement('a'); \
           a.href = u; a.target = '_blank'; a.rel = 'noopener noreferrer'; \
           document.body.appendChild(a); a.click(); a.remove(); }})();"
    ));
}

#[cfg(test)]
mod tests {
    use super::is_web_url;

    /// A URL from the wire is not trusted to be a web address.
    /// `javascript:` through `window.open` opens nothing — it runs, inside the
    /// web view the app is living in.
    #[test]
    fn only_web_addresses_are_handed_out() {
        assert!(is_web_url("https://github.com/me/notes/pull/12"));
        assert!(is_web_url("http://localhost:4300/x"));
        assert!(is_web_url("HTTPS://github.com/me/notes/pull/12"));

        assert!(!is_web_url("javascript:fetch('/settings')"));
        assert!(!is_web_url("file:///etc/passwd"));
        assert!(!is_web_url("data:text/html,<script>x()</script>"));
        assert!(!is_web_url("github.com/me/notes/pull/12"));
        assert!(!is_web_url(""));
    }
}
