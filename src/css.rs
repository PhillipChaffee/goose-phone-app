//! The stylesheet, assembled at compile time.
//!
//! Embedded rather than served, so styling works identically under
//! `cargo run`, `dx serve` and a mobile bundle — there is no file next to the
//! binary on a phone.
//!
//! It is a `concat!` rather than one file because `assets/main.css` is one
//! file five branches would all be appending to at once. A feature that needs
//! rules of its own brings `assets/features/<feature>.css` and replaces its
//! own placeholder line below; nothing else in this list moves. The directory
//! is part of the contract: `docs/audit.js` links `main.css` plus everything
//! in `assets/features/`, so a stylesheet parked anywhere else audits as
//! markup with no rules against it.

/// Every stylesheet in the app, in cascade order. `main.css` is the design
/// system — tokens, chrome, the shared components — so it comes first and
/// everything after it is a feature's own additions.
pub(crate) const STYLES: &str = concat!(
    include_str!("../assets/main.css"),
    // recipes — PR 3 replaces this line

    // skills — PR 4 replaces this line

    // scheduler — PR 5 replaces this line

    // extensions — PR 6 replaces this line
    // session history (PR 7): the search box above the chats list, and the
    // rename sheet's field
    include_str!("../assets/features/session-history.css"),
);
