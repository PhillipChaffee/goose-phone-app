//! The stylesheet, assembled at compile time.
//!
//! Embedded rather than served, so styling works identically under
//! `cargo run`, `dx serve` and a mobile bundle — there is no file next to the
//! binary on a phone.
//!
//! It is a `concat!` rather than one file because `assets/main.css` is one
//! file five branches would all be appending to at once. A feature that needs
//! rules of its own brings `assets/<feature>.css` and replaces its own
//! placeholder line below; nothing else in this list moves.

/// Every stylesheet in the app, in cascade order. `main.css` is the design
/// system — tokens, chrome, the shared components — so it comes first and
/// everything after it is a feature's own additions.
pub(crate) const STYLES: &str = concat!(
    include_str!("../assets/main.css"),
    include_str!("../assets/features/recipes.css"),
    // skills — PR 4 replaces this line

    // scheduler — PR 5 replaces this line

    // extensions — PR 6 replaces this line

    // session history — PR 7 replaces this line
);
