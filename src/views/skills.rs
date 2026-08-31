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
/// (`src/shell/desktop/mod.rs`, `assets/desktop.css`). The `None` arm is the
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
mod tests {
    use goose_acp_client::SourceType;
    use serde_json::Map;

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
