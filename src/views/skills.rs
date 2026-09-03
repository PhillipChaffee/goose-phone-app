//! The Skills screens: a list of what the agent already knows how to do, and
//! one skill's own `SKILL.md` rendered as the prose it was written as.
//!
//! The detail screen is the reason this feature exists. goose Desktop lists
//! skills and shows their metadata; it does not show you what one *says*.
//! `sources/list` has already sent the full markdown inline — see the payload
//! note in `crate::skills` — so rendering it costs nothing but the screen it
//! is on, and reading a skill before invoking it is the difference between
//! trusting the agent and checking it.

use dioxus::prelude::*;
use goose_acp_client::SourceEntry;

use crate::icons::Icon;
use crate::markdown;
use crate::nav::Crumb;
use crate::skills;
use crate::state::{use_app_ctx, AppCtx};
use crate::views::chrome::{ListRow, TopBar};
use crate::views::session_settings::SettingRow;

#[component]
pub fn SkillsView() -> Element {
    let ctx = use_app_ctx();
    let remote = (ctx.skills.list)();
    let conn = (ctx.conn)();
    let connected = conn.is_connected();
    // Reads `conn` and only peeks at the list, so it re-runs when the
    // connection comes up — a user who opens Skills before connecting still
    // gets the fetch — and never when the fetch it started lands.
    use_effect(move || {
        if (ctx.conn)().is_connected() {
            skills::ensure_loaded(&ctx);
        }
    });

    let loading = remote.loading;
    let project_less = skills::project_dir(&ctx.settings.read().working_dir).is_none();

    rsx! {
        TopBar { title: "Skills", conn: true }
        main {
            class: "scroll",
            // On the phone: pull, not a button. A refresh control in the bar
            // would be a 44px invitation to re-download every skill's full
            // text. The desktop has no pull; arriving here re-fetches, and ⌘R
            // does the same re-download on demand (`src/shell/desktop/mod.rs`) —
            // which for Skills is the only refresh there is either way,
            // because `ensure_loaded` is a cache.
            "data-refresh": "skills",
            "data-refreshing": "{loading}",

            if !connected {
                p { class: "error-box",
                    "Not connected. Skills are read from your goose server."
                }
            } else if remote.unsupported {
                // Not a failure and not retryable: this goose has no
                // `sources/list` at all, so there is nothing to offer but the
                // sentence.
                p { class: "empty",
                    "This goose server does not offer skills. It is an older \
                     build, or one started without them."
                }
            } else if let Some(error) = remote.sticky.as_ref() {
                p { class: "error-box", "{error}" }
            } else {
                if project_less {
                    p { class: "hint",
                        "Showing global and built-in skills — set a working \
                         directory in Settings to see this project's."
                    }
                }
                if remote.items.is_empty() {
                    if loading {
                        p { class: "empty", "Loading skills…" }
                    } else {
                        p { class: "empty",
                            "No skills yet. goose loads them from SKILL.md files \
                             under ~/.agents/skills/ or .agents/skills/ in your \
                             project."
                        }
                    }
                }

                ul { class: "session-list",
                    for entry in remote.items.iter() {
                        {skill_row(&ctx, entry)}
                    }
                }
            }
        }
    }
}

/// One row of the list.
///
/// No trailing age — a `SourceEntry` carries no timestamp, and a file's mtime
/// is not something `sources/list` reports, so any age here would be invented.
/// No dot either: a skill has no state to be in. And `actions: vec![]`,
/// because nothing on this screen writes — a row that swipes open onto an
/// empty tray is worse than a row that does not swipe.
fn skill_row(ctx: &AppCtx, entry: &SourceEntry) -> Element {
    // Which row the desktop's detail column came from. Ignored on the phone,
    // where the list is not on screen beside it (`views::chrome::row_is_marked`).
    let selected = ctx
        .skills
        .open
        .read()
        .as_ref()
        .is_some_and(|open| open.path == entry.path);
    let ctx = *ctx;
    let opened = entry.clone();
    rsx! {
        ListRow {
            key: "{entry.path}",
            icon: "sparkle",
            title: "{entry.name}",
            actions: vec![],
            selected,
            on_open: move |()| skills::open(&ctx, opened.clone()),
            div { class: "session-meta",
                for part in meta_parts(entry.scope_label(), entry.supporting_file_count()) {
                    span { key: "{part}", "{part}" }
                }
            }
            if !entry.description.is_empty() {
                div { class: "session-quote", "{entry.description}" }
            }
        }
    }
}

/// The row's second line: where the skill came from, and how much comes with
/// it. Plain data in, plain strings out, so the copy is testable without a
/// component.
fn meta_parts(scope: &str, supporting_files: usize) -> Vec<String> {
    let mut parts = vec![scope.to_owned()];
    if supporting_files > 0 {
        parts.push(file_count(supporting_files));
    }
    parts
}

/// "1 files" is the kind of thing that makes a screen look generated.
fn file_count(files: usize) -> String {
    if files == 1 {
        "1 file".to_owned()
    } else {
        format!("{files} files")
    }
}

/// What the open skill is called, once.
///
/// Read by two things that are never on screen together: the header below,
/// and — on the desktop — the window's own bar, which takes the heading out of
/// the pane and paints it in `.shell-chrome` instead
/// (`src/shell/desktop/mod.rs`, `assets/desktop/`). The `None` arm is the
/// view's own dead-end fallback, for the reason stated there.
pub(crate) fn crumb(ctx: &AppCtx) -> Crumb {
    (ctx.skills.open)().map_or_else(
        || Crumb::plain("Skill"),
        |entry| Crumb::detailed(entry.name.clone(), Some(entry.scope_label().to_owned())),
    )
}

#[component]
pub fn SkillDetailView() -> Element {
    let ctx = use_app_ctx();
    // The one expression the window's bar also reads, so a skill cannot be
    // called one thing in the pane and another in the chrome. Both arms
    // hand `TopBar` a `String` equal to the one the expression they replace
    // produced, into the same prop of the same component — so the phone's
    // captured markup does not move.
    let bar = crumb(&ctx);
    let Some(entry) = (ctx.skills.open)() else {
        // Only reachable if the open skill were cleared while its screen was
        // up. It is not, but a screen with no way back is the one failure
        // this app has already shipped once.
        return rsx! {
            TopBar { title: bar.title, on_back: move |()| skills::close(&ctx) }
            main { class: "scroll", p { class: "empty", "This skill is no longer open." } }
        };
    };
    let connected = (ctx.conn)().is_connected();
    // Frontmatter first: without it the screen opens with the skill's own
    // YAML set as a heading and a rule, because that is what CommonMark makes
    // of `---`.
    let html = markdown::to_html(markdown::strip_frontmatter(&entry.content));
    let name = entry.name.clone();

    rsx! {
        TopBar {
            title: bar.title,
            subtitle: bar.subtitle,
            on_back: move |()| skills::close(&ctx),
        }
        main { class: if connected { "scroll has-fab" } else { "scroll" },
            div { class: "setting-list skill-facts",
                for row in detail_facts(&entry) {
                    div { key: "{row.id}", class: "setting-row fact",
                        span { class: "setting-main",
                            span { class: "setting-name", "{row.name}" }
                            span { class: "setting-value", "{row.value}" }
                            if let Some(note) = row.note {
                                span { class: "setting-note", "{note}" }
                            }
                        }
                    }
                }
            }
            div { class: "md skill-body", dangerous_inner_html: "{html}" }
        }

        if connected {
            button {
                class: "fab",
                onclick: move |_| skills::use_skill(&ctx, &name),
                Icon { name: "message" }
                "Use this skill"
            }
        }
    }
}

/// What the detail screen states about a skill before it shows the document.
///
/// Both are facts in the settings-sheet sense — name, value, and the reason
/// it is a fact — because neither is something the phone can act on. The
/// supporting files in particular: they are absolute paths on the server, and
/// listing them as anything but text would promise a viewer this app does not
/// have.
fn detail_facts(entry: &SourceEntry) -> Vec<SettingRow> {
    let mut rows = vec![if entry.is_editable() {
        SettingRow::fact("kind", "Kind", "Yours", entry.path.clone())
    } else if entry.scope_label() == "Built in" {
        SettingRow::fact("kind", "Kind", "Built in", "Ships with goose.")
    } else {
        // On disk but marked read-only by the server — a real state, and one
        // the path is the explanation for.
        SettingRow::fact("kind", "Kind", "Read-only", entry.path.clone())
    }];
    let files = entry.supporting_file_count();
    if files > 0 {
        let names: Vec<&str> = entry
            .supporting_files
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|path| basename(path))
            .collect();
        rows.push(SettingRow::fact(
            "files",
            "Files",
            file_count(files),
            names.join(" · "),
        ));
    }
    rows
}

/// The last path segment, or the whole thing if there is no separator.
/// goose sends absolute server paths, which are the one string on this screen
/// long enough to push everything else off it.
fn basename(path: &str) -> &str {
    path.rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(path)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "a test that cannot fail on a bad value is worse than one that panics on it"
)]
mod tests {
    use std::cell::RefCell;
    use std::time::Duration;

    use futures_util::{SinkExt as _, StreamExt as _};
    use goose_acp_client::{AcpClient, AcpEvent, ConnectConfig, SourceType};
    use serde_json::{json, Map, Value};
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message;

    use dioxus::html::{
        set_event_converter, PlatformEventData, SerializedHtmlEventConverter, SerializedMouseData,
    };
    use std::any::Any;
    use std::rc::Rc;

    use crate::state::{ChatState, ConnState, Screen as HomeScreen, Tab};
    use crate::testkit::{render, render_seeded};

    use super::*;

    fn entry(source_type: SourceType, writable: Option<bool>, files: &[&str]) -> SourceEntry {
        SourceEntry {
            source_type,
            name: "deploy".to_owned(),
            description: "Ship it".to_owned(),
            content: String::new(),
            path: "/Users/me/work/pilot/.agents/skills/deploy".to_owned(),
            global: false,
            writable,
            supporting_files: (!files.is_empty())
                .then(|| files.iter().map(|f| (*f).to_owned()).collect()),
            properties: None,
            extra: Map::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Fixtures. Seeds are `fn` pointers and cannot capture, so everything a
    // seed needs is a free function it can call.

    /// A project skill with a document and a file beside it.
    fn deploy() -> SourceEntry {
        SourceEntry {
            content: "---\nname: deploy\ndescription: Ship it\n---\n\n# Deploy\n\n\
                      Run `make ship` on the release host.\n"
                .to_owned(),
            ..entry(
                SourceType::Skill,
                Some(true),
                &["/Users/me/work/pilot/.agents/skills/deploy/runbook.md"],
            )
        }
    }

    /// A built-in, which has a different scope word and no path to show.
    fn shipped() -> SourceEntry {
        SourceEntry {
            name: "brainstorm".to_owned(),
            description: String::new(),
            path: "builtin://skills/brainstorm".to_owned(),
            ..entry(SourceType::BuiltinSkill, Some(false), &[])
        }
    }

    fn connect(ctx: &AppCtx) {
        let mut conn = ctx.conn;
        conn.set(ConnState::Connected {
            agent: "goose".to_owned(),
        });
    }

    /// Connected, with a list the server has already answered.
    fn listed(ctx: &AppCtx) {
        connect(ctx);
        let mut list = ctx.skills.list;
        list.write().items = vec![deploy(), shipped()];
    }

    /// The same, plus the working directory that stops the project hint.
    fn listed_in_a_project(ctx: &AppCtx) {
        listed(ctx);
        let mut settings = ctx.settings;
        settings.write().working_dir = "/Users/me/work/pilot".to_owned();
    }

    fn open_deploy(ctx: &AppCtx) {
        let mut open = ctx.skills.open;
        open.set(Some(deploy()));
    }

    fn list_view() -> Element {
        rsx! { super::SkillsView {} }
    }

    fn detail_view() -> Element {
        rsx! { super::SkillDetailView {} }
    }

    // -----------------------------------------------------------------------
    // The list.

    /// Skills are read off the goose server, so an offline phone has nothing
    /// to say about them — and must not say "no skills yet", which reads as a
    /// fact about the server it cannot possibly have.
    #[test]
    fn disconnected_says_where_skills_come_from_rather_than_claiming_there_are_none() {
        let html = render(list_view);
        assert!(
            html.contains("Not connected. Skills are read from your goose server."),
            "a disconnected Skills screen is blank with no explanation"
        );
        assert!(
            !html.contains("No skills yet."),
            "an offline phone is claiming the server has no skills, which it \
             has no way of knowing"
        );
    }

    /// A goose without `sources/list` is not a goose with an empty skills
    /// directory. There is nothing to retry and nothing to add, so the screen
    /// says which of the two it is.
    #[test]
    fn a_server_without_the_feature_is_told_apart_from_one_with_nothing_in_it() {
        fn seed(ctx: &AppCtx) {
            connect(ctx);
            let mut list = ctx.skills.list;
            list.write().unsupported = true;
        }
        let html = render_seeded(seed, list_view);
        assert!(
            html.contains("This goose server does not offer skills."),
            "an unsupported server said nothing about why the screen is blank"
        );
        assert!(
            !html.contains("No skills yet."),
            "an unsupported server is being described as an empty one, which \
             reads as 'go and write one' rather than 'this server cannot'"
        );
    }

    /// A fetch in flight must not be reported as an answer. "No skills yet"
    /// during the first load is a lie the reader acts on — they go and look
    /// for a directory that is not the problem.
    #[test]
    fn a_list_still_loading_does_not_claim_the_server_is_empty() {
        fn seed(ctx: &AppCtx) {
            connect(ctx);
            let mut list = ctx.skills.list;
            list.write().loading = true;
        }
        let html = render_seeded(seed, list_view);
        assert!(
            html.contains("Loading skills…"),
            "the first fetch shows nothing at all while it runs"
        );
        assert!(
            !html.contains("No skills yet."),
            "the empty state is shown while the first fetch is still running"
        );
        assert!(
            html.contains("data-refreshing=\"true\""),
            "the pull-to-refresh control has no way to know a fetch is in \
             flight, so it will never show the spinner"
        );
    }

    /// A failure that leaves nothing readable behind it stays on screen: there
    /// is no list under it to go back to looking at.
    #[test]
    fn a_failed_fetch_says_so_instead_of_showing_an_empty_library() {
        fn seed(ctx: &AppCtx) {
            connect(ctx);
            let mut list = ctx.skills.list;
            list.write().sticky = Some("sources/list timed out".to_owned());
        }
        let html = render_seeded(seed, list_view);
        assert!(
            html.contains("<p class=\"error-box\">sources/list timed out</p>"),
            "a fetch that failed is being shown as a server with no skills"
        );
        assert!(
            !html.contains("No skills yet."),
            "the empty state is on screen underneath the failure that caused it"
        );
    }

    /// The row is the list. It has to carry the name, where the skill came
    /// from, how much comes with it, and goose's own one-line description —
    /// there is no other way to choose between twenty of them.
    #[test]
    fn a_row_says_what_the_skill_is_where_it_is_from_and_what_comes_with_it() {
        let html = render_seeded(listed_in_a_project, list_view);
        assert!(
            html.contains("<div class=\"session-title\">deploy</div>"),
            "the row is not titled with the skill's name"
        );
        assert!(
            html.contains("<span>This project</span>"),
            "a project skill does not say it belongs to this project, so a \
             global one of the same name is indistinguishable from it"
        );
        assert!(
            html.contains("<span>1 file</span>"),
            "the file that comes with the skill is not counted on the row"
        );
        assert!(
            html.contains("<div class=\"session-quote\">Ship it</div>"),
            "goose's own description of the skill never reached the row"
        );
        assert!(
            html.contains("<span>Built in</span>"),
            "a built-in skill is described as one of the reader's own"
        );
        assert!(
            !html.contains("session-actions"),
            "a row that writes nothing is offering a swipe tray, which opens \
             onto nothing at all"
        );
    }

    /// A skill with no description has no second line, rather than an empty
    /// quote block sitting under the name.
    #[test]
    fn a_skill_with_no_description_gets_no_empty_quote_under_it() {
        let html = render_seeded(listed, list_view);
        assert_eq!(
            html.matches("session-quote").count(),
            1,
            "the built-in, whose description is empty, is rendering a blank \
             quote block under its name"
        );
    }

    /// Without a working directory goose still answers — with the global and
    /// built-in skills and none of the project's. That is a materially
    /// different list, so the screen says so rather than letting the reader
    /// conclude their project's skills are missing.
    #[test]
    fn a_list_taken_without_a_project_says_what_is_not_in_it() {
        let without = render_seeded(listed, list_view);
        assert!(
            without.contains("Showing global and built-in skills"),
            "a list fetched with no working directory is presented as the \
             whole truth, so a project's own skills look like they do not exist"
        );
        let within = render_seeded(listed_in_a_project, list_view);
        assert!(
            !within.contains("Showing global and built-in skills"),
            "the hint is on screen for a list that DID carry a project \
             directory, so it is permanent furniture rather than a warning"
        );
    }

    /// On the desktop the list stays beside the pane, so the row it opened
    /// wears the highlight — and only while the pane is actually showing
    /// something.
    #[test]
    fn the_row_is_marked_only_while_the_pane_beside_it_is_open() {
        fn opened(ctx: &AppCtx) {
            listed(ctx);
            open_deploy(ctx);
            let mut tab = ctx.tab;
            tab.set(Tab::Skills);
            let mut screen = ctx.skills.screen;
            screen.set(skills::Screen::Detail);
        }
        // The open skill outlives the screen that set it, which is the whole
        // reason the mark cannot be read off it alone.
        fn back_at_the_list(ctx: &AppCtx) {
            listed(ctx);
            open_deploy(ctx);
            let mut tab = ctx.tab;
            tab.set(Tab::Skills);
        }
        let showing = render_seeded(opened, list_view);
        assert!(
            showing.contains("class=\"session-item on\""),
            "the row the detail pane is showing is not marked, so the \
             desktop's two columns give no clue which one they are about"
        );
        assert_eq!(
            showing.matches("session-item on").count(),
            1,
            "more than one row is marked as the one the pane is showing"
        );
        let closed = render_seeded(back_at_the_list, list_view);
        assert!(
            !closed.contains("session-item on"),
            "a row kept the highlight after the pane closed, so the list \
             claims to be showing something the pane says is not open"
        );
    }

    // -----------------------------------------------------------------------
    // The detail.

    /// The detail screen is why this feature exists: goose Desktop lists
    /// skills and never shows you what one SAYS. So the document has to be on
    /// screen, as prose — and its frontmatter must not be, because `CommonMark`
    /// turns a leading `---` block into a heading and a rule and the screen
    /// would open on the skill's own YAML set in 24px type.
    #[test]
    fn the_detail_renders_the_document_and_not_its_frontmatter() {
        fn seed(ctx: &AppCtx) {
            connect(ctx);
            open_deploy(ctx);
        }
        let html = render_seeded(seed, detail_view);
        assert!(
            html.contains("<h1>Deploy</h1>"),
            "the skill's own document is not on the screen that exists to show it"
        );
        assert!(
            html.contains("<code>make ship</code>"),
            "the document is being shown as raw markdown rather than as prose"
        );
        assert!(
            !html.contains("description: Ship it"),
            "the skill's YAML frontmatter is being rendered as the first thing \
             on the screen"
        );
        assert!(
            html.contains("<h1 class=\"title ellipsis\">deploy</h1>")
                && html.contains("<span class=\"subtitle ellipsis\">This project</span>"),
            "the bar over the open skill has lost its name or where it came from"
        );
        assert!(
            html.contains("<span class=\"setting-value\">Yours</span>")
                && html.contains("/Users/me/work/pilot/.agents/skills/deploy"),
            "a skill the reader owns does not say so, or does not say where it \
             lives — which is the only way to go and edit it"
        );
        assert!(
            html.contains("<span class=\"setting-value\">1 file</span>")
                && html.contains("runbook.md"),
            "the file that ships beside the skill is not named, so there is no \
             way to know the agent has more than the document in front of it"
        );
    }

    /// The "use this skill" button is the phone's whole reason for showing a
    /// skill at all — and it can only be offered when there is a server to
    /// send the message to. Offered while disconnected it is a control that
    /// can only fail, and design rule 11 says do not draw it.
    #[test]
    fn the_use_button_is_only_offered_when_there_is_somewhere_to_send_it() {
        fn seed(ctx: &AppCtx) {
            connect(ctx);
            open_deploy(ctx);
        }
        let live = render_seeded(seed, detail_view);
        assert!(
            live.contains("Use this skill") && live.contains("class=\"fab\""),
            "a connected reader has no way to turn the skill they are reading \
             into a message"
        );
        assert!(
            live.contains("class=\"scroll has-fab\""),
            "the document has no room reserved for the button floating over \
             it, so its last paragraph sits underneath the button"
        );
        let offline = render_seeded(open_deploy, detail_view);
        assert!(
            !offline.contains("Use this skill"),
            "the use button is live while disconnected, so pressing it can \
             only ever fail"
        );
        assert!(
            offline.contains("class=\"scroll\""),
            "the document is still reserving room for a button that is not \
             there, leaving a gap at the end of every skill read offline"
        );
    }

    /// Only reachable if the open skill were cleared while its screen was up —
    /// but a screen with no way back is the one failure this app has already
    /// shipped once, so the bar still has a word in it and a chevron under it.
    #[test]
    fn a_detail_with_no_skill_still_has_a_title_and_a_way_back() {
        let html = render(detail_view);
        assert!(
            html.contains("This skill is no longer open."),
            "a cleared skill leaves a blank detail page"
        );
        assert!(
            html.contains("<h1 class=\"title ellipsis\">Skill</h1>"),
            "the bar over a cleared skill has no title at all"
        );
        assert!(
            html.contains("class=\"icon-btn back\""),
            "the dead-end screen has no back chevron, so the only way off it \
             is the drawer"
        );
    }

    /// And the chevron on that screen has to WORK. A back button that renders
    /// but does not navigate is the same dead end with a control drawn over
    /// it — and this arm's handler is a second copy of the one on the live
    /// screen, so nothing else in the suite touches it.
    #[test]
    fn the_chevron_over_a_cleared_skill_really_goes_back() {
        fn screen() -> Element {
            let ctx = crate::state::use_app_ctx_provider();
            use_hook(|| {
                // The vanished state exactly: the detail screen is up and the
                // skill it was showing is gone.
                let mut sc = ctx.skills.screen;
                sc.set(skills::Screen::Detail);
            });
            rsx! {
                super::SkillDetailView {}
                if (ctx.skills.screen)() == skills::Screen::List {
                    p { "back at the list" }
                }
            }
        }
        assert_eq!(
            taps_that(screen, |html| html.contains("back at the list")),
            1,
            "the screen a cleared skill leaves behind has no single control \
             that returns to the list, so the reader is stuck on a page that \
             says only that there is nothing on it"
        );
    }

    /// `crumb` is read by two things that are never on screen together — this
    /// pane's own header and, on the desktop, the window's bar — so a skill
    /// cannot be called one thing in the pane and another in the chrome.
    #[test]
    fn the_name_the_window_bar_reads_is_the_name_the_pane_reads() {
        fn probe() -> Element {
            let ctx = crate::state::use_app_ctx();
            let crumb = crumb(&ctx);
            let subtitle = crumb.subtitle.unwrap_or_default();
            rsx! { p { "[{crumb.title}][{subtitle}]" } }
        }
        assert!(
            render_seeded(open_deploy, probe).contains("<p>[deploy][This project]</p>"),
            "an open skill's crumb lost either its name or where it came from"
        );
        assert!(
            render(probe).contains("<p>[Skill][]</p>"),
            "with no skill open the window's bar has no word in it at all"
        );
    }

    // -----------------------------------------------------------------------
    // What the controls do when they are tapped.

    /// An `ElementId` past the end is ignored rather than fatal
    /// (`Runtime::handle_event` does a `get`), so this only has to be larger
    /// than any screen here.
    const EVERY_ELEMENT: u32 = 80;

    /// Mount, click one element, and hand back the markup that produced.
    ///
    /// WHICH ELEMENT IS TAPPED IS NOT GUESSED AT: Dioxus addresses an element
    /// by an `ElementId` assigned in creation order and nothing in the markup
    /// maps back to one, so every element is tapped in its own fresh mount and
    /// the assertion is on how many of them did the thing — the same shape
    /// `views::extensions`'s tap tests use, and for the same reason.
    fn tap(app: fn() -> Element, target: u32) -> String {
        let _ = crate::testkit::storage_dir();
        set_event_converter(Box::new(SerializedHtmlEventConverter));
        // A handler that reaches `show_toast` spawns a `tokio::time::sleep`,
        // and Dioxus polls it during the re-render below.
        let reactor = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("a current-thread runtime for the toast timer to register with");
        let _guard = reactor.enter();
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        let data: Box<dyn Any> = Box::new(SerializedMouseData::default());
        let event: Rc<dyn Any> = Rc::new(PlatformEventData::new(data));
        dom.runtime().handle_event(
            "click",
            dioxus::dioxus_core::Event::new(event, false),
            dioxus::dioxus_core::ElementId(target as usize),
        );
        dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        dioxus_ssr::render(&dom)
    }

    /// How many of the screen's elements, tapped one at a time, leave it in a
    /// state `outcome` recognises.
    fn taps_that(app: fn() -> Element, outcome: fn(&str) -> bool) -> usize {
        (1..=EVERY_ELEMENT)
            .filter(|target| outcome(&tap(app, *target)))
            .count()
    }

    /// Tapping a row opens that skill — and the skill it opens is the row that
    /// was tapped, not whichever one the signal happened to hold.
    #[test]
    fn tapping_a_row_opens_that_skill_and_not_another() {
        fn screen() -> Element {
            let ctx = crate::state::use_app_ctx_provider();
            use_hook(|| listed(&ctx));
            rsx! {
                super::SkillsView {}
                if (ctx.skills.screen)() == skills::Screen::Detail {
                    super::SkillDetailView {}
                }
            }
        }
        let opened: Vec<String> = (1..=EVERY_ELEMENT)
            .map(|target| tap(screen, target))
            .filter(|html| html.contains("class=\"md skill-body\""))
            .collect();
        assert_eq!(
            opened.len(),
            2,
            "a two-row list does not have exactly two controls that open a \
             skill — a row has stopped opening, or something that is not a row \
             opens one"
        );
        assert!(
            opened.iter().any(|html| html.contains("<h1>Deploy</h1>")),
            "no row opens the deploy skill's own document"
        );
        assert!(
            opened
                .iter()
                .any(|html| html.contains("<h1 class=\"title ellipsis\">brainstorm</h1>")),
            "both rows open the same skill, so which one you tapped makes no \
             difference to what you get"
        );
    }

    /// The detail's back chevron returns to the list AND drops the skill it
    /// was holding — the full text of a `SKILL.md` is the largest thing this
    /// feature keeps, and nothing off-screen needs a copy of it.
    #[test]
    fn the_detail_goes_back_to_the_list_and_lets_go_of_the_document() {
        fn screen() -> Element {
            let ctx = crate::state::use_app_ctx_provider();
            use_hook(|| {
                listed(&ctx);
                open_deploy(&ctx);
                let mut sc = ctx.skills.screen;
                sc.set(skills::Screen::Detail);
            });
            let held = (ctx.skills.open)().is_some();
            rsx! {
                super::SkillDetailView {}
                if (ctx.skills.screen)() == skills::Screen::List {
                    p { "back at the list, holding={held}" }
                }
            }
        }
        assert_eq!(
            taps_that(screen, |html| html.contains("back at the list")),
            1,
            "the open skill has no single control that returns to the list, so \
             the back chevron is either gone or no longer the only thing that \
             goes back"
        );
        let back = (1..=EVERY_ELEMENT)
            .map(|target| tap(screen, target))
            .find(|html| html.contains("back at the list"))
            .expect("no tap went back to the list");
        assert!(
            back.contains("holding=false"),
            "the list is back on screen with the whole of the last skill's \
             SKILL.md still held behind it"
        );
    }

    /// "Use this skill" is the one thing this screen does. It fills the
    /// composer rather than sending — what the skill needs is the half only
    /// the reader knows — and it does it in the chat that is already open,
    /// which is where the context they have built up lives.
    #[test]
    fn the_use_button_types_the_invocation_into_the_open_chat() {
        fn screen() -> Element {
            let ctx = crate::state::use_app_ctx_provider();
            use_hook(|| {
                connect(&ctx);
                open_deploy(&ctx);
                let mut chat = ctx.chat;
                chat.set(ChatState {
                    session_id: Some("s-1".to_owned()),
                    ..ChatState::default()
                });
            });
            let draft = (ctx.chat_draft)();
            let home = (ctx.tab)() == Tab::Home && (ctx.screen)() == HomeScreen::Chat;
            rsx! {
                super::SkillDetailView {}
                p { class: "probe", "[{draft}][{home}]" }
            }
        }
        assert_eq!(
            taps_that(screen, |html| html.contains("[Use the deploy skill: ]")),
            1,
            "the skill screen does not have exactly one control that types the \
             invocation into the composer"
        );
        let used = (1..=EVERY_ELEMENT)
            .map(|target| tap(screen, target))
            .find(|html| html.contains("[Use the deploy skill: ]"))
            .expect("nothing on the screen used the skill");
        assert!(
            used.contains("[Use the deploy skill: ][true]"),
            "the invocation was typed but the reader was left on the skill's \
             own screen, so the composer holding it is nowhere in sight"
        );
    }

    // -----------------------------------------------------------------------
    // Against a real server.
    //
    // Everything above seeds the list by hand, which leaves the one thing this
    // screen does on arrival — fetch — outside the suite entirely. `AcpClient`
    // has no constructor but `connect`, so the only way to drive it is to put
    // a server in front of it: a plain-`ws://` JSON-RPC listener on a loopback
    // port. `ws_url` only reaches for TLS on an `https://` base, so `http://`
    // here means no certificate and no fingerprint.

    thread_local! {
        /// The context [`Live`]'s mount built, so a test can reach it.
        static PUBLISHED: RefCell<Option<AppCtx>> = const { RefCell::new(None) };
        /// The client [`Live`] connected, for the mount's own hook to adopt.
        static CLIENT: RefCell<Option<AcpClient>> = const { RefCell::new(None) };
    }

    /// The Skills list over a real context, published so a test can connect it
    /// after the fact — which is the case the effect exists for.
    fn published_list() -> Element {
        let ctx = crate::state::use_app_ctx_provider();
        use_hook(move || PUBLISHED.with(|slot| *slot.borrow_mut() = Some(ctx)));
        rsx! { super::SkillsView {} }
    }

    /// Two skills, in the wire shape `sources/list` answers with.
    fn wire(method: &str, params: &Value) -> Value {
        assert_eq!(method, "_goose/unstable/sources/list");
        let kind = params
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let sources = if kind == "builtinSkill" {
            vec![shipped()]
        } else {
            vec![deploy()]
        };
        json!({ "sources": sources })
    }

    struct Live {
        dom: VirtualDom,
        rt: tokio::runtime::Runtime,
        ctx: AppCtx,
        /// The connection's event stream, parked here for its lifetime: the
        /// client's actor gives up the socket when this end goes away.
        _events: tokio::sync::mpsc::Receiver<AcpEvent>,
    }

    impl Live {
        /// A mounted Skills screen, disconnected, with a live client waiting in
        /// [`CLIENT`] for [`Self::connect`] to hand it over.
        fn new(script: fn(&str, &Value) -> Value) -> Self {
            let _ = crate::testkit::storage_dir();
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("a tokio runtime for the socket to live on");
            let base_url = serve(&rt, script);
            let cfg = ConnectConfig {
                base_url,
                secret: String::new(),
                fingerprint: None,
            };
            let (client, events, _info) = rt
                .block_on(AcpClient::connect(&cfg))
                .expect("the mock server accepted the handshake");
            CLIENT.with(|slot| *slot.borrow_mut() = Some(client));
            let mut dom = VirtualDom::new(published_list);
            rt.block_on(async { dom.rebuild_in_place() });
            let ctx = PUBLISHED
                .with(|slot| *slot.borrow())
                .expect("the mount rendered, so it published its context");
            Self {
                dom,
                rt,
                ctx,
                _events: events,
            }
        }

        /// Read or write the context. Signals belong to the virtual DOM's
        /// runtime and panic outside it, so every touch goes through here.
        fn with<T>(&self, f: impl FnOnce(&AppCtx) -> T) -> T {
            let ctx = self.ctx;
            self.dom.in_runtime(|| f(&ctx))
        }

        /// The connection coming up under a screen that is already on.
        fn connect(&mut self) {
            self.with(|ctx| {
                ctx.client
                    .clone()
                    .set(CLIENT.with(|slot| slot.borrow().clone()));
                let mut conn = ctx.conn;
                conn.set(ConnState::Connected {
                    agent: "goose".to_owned(),
                });
            });
            self.settle();
        }

        /// Let the queued Dioxus tasks — and the socket under them — run.
        ///
        /// Bounded on purpose: a screen whose tasks never settle must not hang
        /// the suite, and 400ms of idle against a loopback round trip is room
        /// to spare rather than a wall-clock wait.
        fn settle(&mut self) {
            let dom = &mut self.dom;
            self.rt.block_on(async {
                for _ in 0..40 {
                    let _ =
                        tokio::time::timeout(Duration::from_millis(10), dom.wait_for_work()).await;
                    dom.render_immediate_to_vec();
                }
            });
        }

        fn markup(&self) -> String {
            dioxus_ssr::render(&self.dom)
        }
    }

    /// A JSON-RPC server on a loopback port, answering `script`.
    fn serve(rt: &tokio::runtime::Runtime, script: fn(&str, &Value) -> Value) -> String {
        let listener = rt.block_on(async {
            TcpListener::bind("127.0.0.1:0")
                .await
                .expect("a loopback port")
        });
        let port = listener
            .local_addr()
            .expect("the listener's own address")
            .port();
        rt.spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                tokio::spawn(async move { session_loop(socket, script).await });
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    async fn session_loop(socket: tokio::net::TcpStream, script: fn(&str, &Value) -> Value) {
        let Ok(ws) = tokio_tungstenite::accept_async(socket).await else {
            return;
        };
        let (mut sink, mut stream) = ws.split();
        while let Some(Ok(msg)) = stream.next().await {
            let Message::Text(text) = msg else { continue };
            let Ok(frame) = serde_json::from_str::<Value>(text.as_str()) else {
                continue;
            };
            let Some(id) = frame.get("id").cloned() else {
                continue;
            };
            let method = frame
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let params = frame.get("params").cloned().unwrap_or(Value::Null);
            let result = if method == "initialize" {
                json!({
                    "protocolVersion": 1,
                    "agentInfo": { "name": "mock", "version": "0" },
                })
            } else {
                script(&method, &params)
            };
            let reply = json!({ "jsonrpc": "2.0", "id": id, "result": result });
            if sink
                .send(Message::Text(reply.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    }

    /// THE EFFECT IS THE ONLY FETCH THIS SCREEN HAS. There is no refresh
    /// button, no timer and — on the desktop — no pull, so a screen that is
    /// already up when the connection comes back has to notice: the effect
    /// reads `conn` and only *peeks* at the list, so it re-runs on connect and
    /// not on the answer it started. Break that and Skills stays permanently
    /// empty for anyone who opened it before typing a server URL, with no
    /// control anywhere on the phone to make it try again.
    #[test]
    fn a_screen_opened_before_connecting_fills_itself_in_when_the_socket_comes_up() {
        let mut live = Live::new(wire);
        live.settle();
        let before = live.markup();
        assert!(
            before.contains("Not connected. Skills are read from your goose server."),
            "the screen did not start disconnected, so what follows proves \
             nothing about the connection arriving"
        );
        assert!(
            !before.contains("deploy"),
            "the list was already full before anything was connected"
        );

        live.connect();
        let after = live.markup();
        assert!(
            after.contains("<div class=\"session-title\">deploy</div>"),
            "the connection came up under an open Skills screen and nothing \
             fetched, so the list stays empty with no control on the phone \
             that could ever fill it"
        );
        assert!(
            after.contains("<div class=\"session-title\">brainstorm</div>"),
            "only one of the two kinds of source reached the screen, so \
             built-in skills are invisible"
        );
        assert!(
            !after.contains("data-refreshing=\"true\""),
            "the fetch finished but the screen is still showing itself as \
             loading, so the pull spinner never stops"
        );
    }

    #[test]
    fn a_skill_with_nothing_beside_it_says_only_where_it_is_from() {
        assert_eq!(meta_parts("Global", 0), ["Global"]);
    }

    #[test]
    fn one_file_is_singular() {
        assert_eq!(meta_parts("This project", 1), ["This project", "1 file"]);
        assert_eq!(meta_parts("This project", 4), ["This project", "4 files"]);
    }

    #[test]
    fn a_built_in_is_named_as_one_and_shows_no_path() {
        let rows = detail_facts(&entry(SourceType::BuiltinSkill, Some(true), &[]));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value, "Built in");
        assert_eq!(rows[0].note.as_deref(), Some("Ships with goose."));
    }

    #[test]
    fn a_writable_skill_shows_where_it_lives() {
        let rows = detail_facts(&entry(SourceType::Skill, Some(true), &[]));
        assert_eq!(rows[0].value, "Yours");
        assert_eq!(
            rows[0].note.as_deref(),
            Some("/Users/me/work/pilot/.agents/skills/deploy")
        );
    }

    /// A filesystem skill the server marked read-only is neither "yours" nor
    /// shipped with goose, and saying either would be a lie about what the
    /// server would accept.
    #[test]
    fn a_read_only_skill_says_so() {
        let rows = detail_facts(&entry(SourceType::Skill, Some(false), &[]));
        assert_eq!(rows[0].value, "Read-only");
    }

    #[test]
    fn supporting_files_are_counted_and_named_by_basename() {
        let rows = detail_facts(&entry(
            SourceType::Skill,
            Some(true),
            &[
                "/Users/me/work/pilot/.agents/skills/deploy/runbook.md",
                "/Users/me/work/pilot/.agents/skills/deploy/rollback.sh",
            ],
        ));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].value, "2 files");
        assert_eq!(rows[1].note.as_deref(), Some("runbook.md · rollback.sh"));
    }

    #[test]
    fn basename_survives_the_shapes_a_path_arrives_in() {
        assert_eq!(basename("/a/b/c.md"), "c.md");
        assert_eq!(basename("/a/b/dir/"), "dir");
        assert_eq!(basename("plain.md"), "plain.md");
        assert_eq!(basename(""), "");
    }
}
