//! Mounting a view and PRESSING something in it.
//!
//! [`crate::testkit`] renders; this one clicks and types. The difference is
//! the half of the app that only an event reaches: every `onclick` in
//! `views/mod.rs` and in `shell/mod.rs`'s `render_destination` is a closure
//! that a render pass builds and never calls, so a suite that only renders
//! measures the markup and none of the behaviour behind it. A destination
//! button that navigated nowhere, a Delete that stopped calling its handler
//! and a rename that saved the old title would all render identically.
//!
//! WHICH ELEMENT. `dioxus_ssr::pre_render` numbers every element it writes
//! with `data-node-hydration="N,click:…"`, and the walk below pairs those
//! numbers with the live `ElementId`s in the same order — it is
//! `dioxus-web`'s own `rehydrate`, minus suspense and `onmounted`. So a test
//! names a control the way a reader would ("the one with this title"), and
//! the press lands on the element the markup it asserted about came from.
//!
//! WHICH PAYLOAD. `dioxus-html` routes every listener through a process-global
//! `HtmlEventConverter` that a renderer installs at launch, and without one
//! the `.unwrap()` inside `ListenerCallback` panics.
//! `SerializedHtmlEventConverter` is the converter `dioxus-desktop` installs,
//! so a press here is converted by the code the shipped app uses.
//!
//! A second copy of the same idea already lives in `views/chat.rs`, private to
//! that module and wired to that screen's seeds. This is the one three modules
//! share; merging them is a tidy-up for a pass that owns both files.

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "test scaffolding: a press that cannot find its button has \
              nothing to assert, so failing loudly there IS the check"
)]

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;
use std::task::{Context, Poll};

use dioxus::dioxus_core::{
    DynamicNode, ElementId, NoOpMutations, ScopeState, TemplateAttribute, TemplateNode, VNode,
};
use dioxus::html::{
    Code, Key, Location, Modifiers, PlatformEventData, SerializedFormData,
    SerializedHtmlEventConverter, SerializedKeyboardData, SerializedMouseData,
};
use dioxus::prelude::document::{Document, Eval, EvalError, Evaluator};
use dioxus::prelude::*;

use crate::state::AppCtx;

/// Every script the app handed to `document::eval`, and what the JS end says
/// back.
///
/// Half of `src/viewport.rs` is a `document::eval` and nothing else: the
/// viewport mirror, the transcript-bottom listener, the jump to the bottom and
/// the file picker all *are* the script they install. Without a document in
/// context Dioxus falls back to `NoOpDocument`, which throws the script away
/// and answers `Unsupported` — so a test would run those functions, assert
/// nothing, and move the coverage number for a hook that had stopped
/// installing anything at all.
///
/// [`Self::inbox`] is the other direction, and it is what makes
/// `use_file_picker`'s loop body reachable: the real picker sends a JSON
/// payload up from the webview, and this replays one so the handler that reads
/// it actually runs.
#[derive(Clone, Default)]
pub(crate) struct Js {
    scripts: Rc<RefCell<Vec<String>>>,
    inbox: Rc<RefCell<Vec<serde_json::Value>>>,
}

impl Js {
    /// Put this recorder in context, so every `document::eval` under the
    /// calling component reaches it. Call it from a component body.
    pub(crate) fn install(&self) {
        let doc: Rc<dyn Document> = Rc::new(self.clone());
        provide_context(doc);
    }

    /// Queue one message for the app's next `eval.recv()`, the way the
    /// webview's own JS would send it.
    pub(crate) fn will_send(&self, payload: &str) {
        self.inbox
            .borrow_mut()
            .push(serde_json::Value::String(payload.to_owned()));
    }

    /// Every script evaluated so far, oldest first.
    pub(crate) fn scripts(&self) -> Vec<String> {
        self.scripts.borrow().clone()
    }

    /// Forget everything recorded. A recorder reached through a
    /// `thread_local` — which is how a non-capturing `fn() -> Element` probe
    /// gets hold of one — outlives the mount that used it, so a test that did
    /// not start from empty would be asserting about the previous one.
    pub(crate) fn clear(&self) {
        self.scripts.borrow_mut().clear();
        self.inbox.borrow_mut().clear();
    }

    /// The one script containing `needle`, or a panic naming what was
    /// installed instead — an assertion about a script that was never
    /// evaluated is the failure this exists to report.
    pub(crate) fn script_with(&self, needle: &str) -> String {
        let all = self.scripts();
        all.iter()
            .find(|script| script.contains(needle))
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "no script containing {needle:?} was evaluated; {} were: {all:?}",
                    all.len()
                )
            })
    }
}

impl Document for Js {
    fn eval(&self, js: String) -> Eval {
        self.scripts.borrow_mut().push(js);
        let owner = Owner::default();
        let evaluator: Box<dyn Evaluator> = Box::new(Replay {
            inbox: Rc::clone(&self.inbox),
        });
        let handle = owner.insert(evaluator);
        // Leaked on purpose, and it is the only arrangement that works. A
        // dropped `Owner` frees the slot, so an evaluator whose owner went
        // away answers `Finished` instead of the payload this queued — and an
        // owner *kept* in the recorder is dropped when the `thread_local`
        // holding it is torn down, which is after `generational_box`'s own
        // thread-local runtime has gone and aborts the process. A handful of
        // slots per test run is the right price.
        std::mem::forget(owner);
        Eval::new(handle)
    }
}

/// The JS end of one `eval`: it hands over whatever was queued, then reports
/// that there is nothing more — which is how the real evaluator ends a
/// `while let Ok(..) = eval.recv()` loop when the webview goes away.
struct Replay {
    inbox: Rc<RefCell<Vec<serde_json::Value>>>,
}

impl Evaluator for Replay {
    fn send(&self, _data: serde_json::Value) -> Result<(), EvalError> {
        Ok(())
    }

    fn poll_recv(&mut self, _cx: &mut Context<'_>) -> Poll<Result<serde_json::Value, EvalError>> {
        let next = if self.inbox.borrow().is_empty() {
            None
        } else {
            Some(self.inbox.borrow_mut().remove(0))
        };
        Poll::Ready(next.ok_or(EvalError::Finished))
    }

    fn poll_join(&mut self, _cx: &mut Context<'_>) -> Poll<Result<serde_json::Value, EvalError>> {
        Poll::Ready(Err(EvalError::Finished))
    }
}

/// The two process-wide things a press needs, set up exactly once, and the
/// runtime handed back because that is the half a press has to enter.
///
/// Both have to be process-wide rather than per-mount: `cargo test` runs these
/// on every core at once, and an event converter reinstalled under a reader —
/// or a `tokio` runtime built and dropped around every press — wedges the
/// binary while every individual test still passes on its own.
///
/// The runtime is entered for a press and DRIVEN by [`Pressable::settle`].
/// `show_toast` arms a `tokio::time::sleep` to take the toast away again and
/// constructing that without a runtime panics; a socket needs more than that,
/// which is why it is `enable_all` and why [`runtime`] hands it out — a client
/// connected on some other runtime would have its actor parked on a reactor
/// nothing here ever polls.
fn install_once() -> &'static tokio::runtime::Runtime {
    static TIMERS: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    TIMERS.get_or_init(|| {
        dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread tokio runtime for the toast timer")
    })
}

/// The one runtime a mounted view's async work runs on.
///
/// Every socket a test stands up has to be built and connected on this one:
/// [`Pressable::settle`] drives it, and a task spawned anywhere else makes no
/// progress at all while the virtual DOM is being polled.
pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    install_once()
}

/// One mounted view at a time.
///
/// `dioxus-sdk-storage` keeps one process-global sender per storage key, and
/// the ask journal behind `use_app_ctx_provider` is on it — so two mounts
/// running task queues at once can feed each other and never settle. Every
/// test here holds this for its whole life.
///
/// A poisoned lock is taken anyway: a test that panicked while holding it has
/// already reported the thing it exists to report, and taking the rest of the
/// module down behind it would only hide which one broke.
pub(crate) fn alone() -> std::sync::MutexGuard<'static, ()> {
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What a harness mounts: the state to put under the view, and the view.
#[derive(Clone, Copy)]
struct Mount {
    seed: fn(&AppCtx),
    view: fn() -> Element,
}

/// Never memoise: a harness mounts once and is dropped, and comparing `fn`
/// pointers is meaningless (see `crate::testkit`).
impl PartialEq for Mount {
    fn eq(&self, _other: &Self) -> bool {
        false
    }
}

thread_local! {
    /// The context the harness provided, so a test can read the state a press
    /// changed without going hunting for it in the markup.
    static PUBLISHED: std::cell::Cell<Option<AppCtx>> = const { std::cell::Cell::new(None) };
}

#[expect(
    non_snake_case,
    reason = "a Dioxus component is named like a component, not like a fn"
)]
fn Harness(props: Mount) -> Element {
    let ctx = crate::state::use_app_ctx_provider();
    use_hook(|| {
        PUBLISHED.with(|slot| slot.set(Some(ctx)));
        (props.seed)(&ctx);
    });
    (props.view)()
}

/// A mounted view with a way in.
pub(crate) struct Pressable {
    dom: VirtualDom,
    ctx: AppCtx,
    /// The render with `data-node-hydration` left in, and the elements it
    /// numbers, in the same order. Recomputed after every event, because an
    /// event that opens a sheet creates elements.
    hydrated: String,
    ids: Vec<ElementId>,
}

/// The element behind each `data-node-hydration` number, in that order.
fn hydration_ids(dom: &VirtualDom) -> Vec<ElementId> {
    let mut ids = Vec::new();
    walk_scope(dom, dom.base_scope(), &mut ids);
    ids
}

fn walk_scope(dom: &VirtualDom, scope: &ScopeState, ids: &mut Vec<ElementId>) {
    walk_vnode(dom, scope.root_node(), ids);
}

fn walk_vnode(dom: &VirtualDom, vnode: &VNode, ids: &mut Vec<ElementId>) {
    for (index, root) in vnode.template.roots.iter().enumerate() {
        walk_template_node(dom, vnode, root, ids, vnode.mounted_root(index, dom));
    }
}

fn walk_template_node(
    dom: &VirtualDom,
    vnode: &VNode,
    node: &TemplateNode,
    ids: &mut Vec<ElementId>,
    root_id: Option<ElementId>,
) {
    match node {
        TemplateNode::Element {
            children, attrs, ..
        } => {
            // An element is numbered when it is a template root or carries a
            // dynamic attribute, which is exactly when the SSR renderer writes
            // `data-node-hydration` on it.
            let mut mounted = root_id;
            for attr in *attrs {
                if let TemplateAttribute::Dynamic { id } = attr {
                    if let Some(id) = vnode.mounted_dynamic_attribute(*id, dom) {
                        mounted = Some(id);
                    }
                }
            }
            if let Some(id) = mounted {
                ids.push(id);
            }
            for child in *children {
                walk_template_node(dom, vnode, child, ids, None);
            }
        }
        TemplateNode::Dynamic { id } => {
            walk_dynamic_node(dom, vnode, &vnode.dynamic_nodes[*id], *id, ids);
        }
        TemplateNode::Text { .. } => {
            if let Some(id) = root_id {
                ids.push(id);
            }
        }
    }
}

fn walk_dynamic_node(
    dom: &VirtualDom,
    vnode: &VNode,
    dynamic: &DynamicNode,
    index: usize,
    ids: &mut Vec<ElementId>,
) {
    match dynamic {
        DynamicNode::Text(_) | DynamicNode::Placeholder(_) => {
            if let Some(id) = vnode.mounted_dynamic_node(index, dom) {
                ids.push(id);
            }
        }
        DynamicNode::Component(component) => {
            if let Some(scope) = component.mounted_scope(index, vnode, dom) {
                walk_scope(dom, scope, ids);
            }
        }
        DynamicNode::Fragment(fragment) => {
            for node in fragment {
                walk_vnode(dom, node, ids);
            }
        }
    }
}

impl Pressable {
    pub(crate) fn mount(seed: fn(&AppCtx), view: fn() -> Element) -> Self {
        // The same one owner as `crate::testkit`: `set_directory` writes a
        // process-wide `OnceLock` and unwraps, so a second caller panics.
        let _ = crate::testkit::storage_dir();
        let _ = install_once();
        let mut dom = VirtualDom::new_with_props(Harness, Mount { seed, view });
        dom.rebuild_in_place();
        let ctx = PUBLISHED.with(std::cell::Cell::take).expect(
            "the harness never published a context — Dioxus swallows a panic \
             thrown during render, so the provider itself failed",
        );
        let mut screen = Self {
            dom,
            ctx,
            hydrated: String::new(),
            ids: Vec::new(),
        };
        screen.reread();
        screen
    }

    fn reread(&mut self) {
        self.hydrated = dioxus_ssr::pre_render(&self.dom);
        self.ids = hydration_ids(&self.dom);
    }

    /// What the screen says right now.
    pub(crate) fn markup(&self) -> String {
        dioxus_ssr::render(&self.dom)
    }

    /// Let the mount's effects — and whatever they spawned — run.
    ///
    /// A `use_effect` has not fired when the first render pass ends, so
    /// everything `src/viewport.rs` installs happens after it. The loop is
    /// bounded for `testkit::render_settled`'s reason: a view whose tasks never
    /// settle must not be able to hang the suite.
    pub(crate) fn settle(&mut self) {
        const PASSES: usize = 8;
        const SLICE: std::time::Duration = std::time::Duration::from_millis(20);

        let dom = &mut self.dom;
        install_once().block_on(async {
            for _ in 0..PASSES {
                let _ = tokio::time::timeout(SLICE, dom.wait_for_work()).await;
                dom.render_immediate_to_vec();
            }
        });
        self.reread();
    }

    /// Read or write the context the view is mounted over. Signals belong to
    /// the virtual DOM's runtime and panic outside it, so every touch of one
    /// goes through here.
    pub(crate) fn with<T>(&self, f: impl FnOnce(&AppCtx) -> T) -> T {
        let ctx = self.ctx;
        self.dom.in_runtime(|| f(&ctx))
    }

    /// The `ElementId` of the first element whose opening tag contains
    /// `needle` and which carries an `event` listener.
    fn locate(&self, event: &str, needle: &str) -> ElementId {
        const MARK: &str = " data-node-hydration=\"";
        let mut at = 0;
        while let Some(rel) = self.hydrated[at..].find(MARK) {
            let start = at + rel;
            let value = start + MARK.len();
            let end = value
                + self.hydrated[value..]
                    .find('"')
                    .expect("an unterminated data-node-hydration attribute");
            let tag_start = self.hydrated[..start].rfind('<').unwrap_or(0);
            let tag = &self.hydrated[tag_start..start];
            let mut parts = self.hydrated[value..end].split(',');
            let number: usize = parts
                .next()
                .and_then(|n| n.parse().ok())
                .expect("a data-node-hydration number that is not a number");
            if parts.any(|l| l.split(':').next() == Some(event)) && tag.contains(needle) {
                return *self.ids.get(number).expect(
                    "the markup numbers an element the hydration walk never \
                     reached, so the two are out of step and a press would \
                     land somewhere else entirely",
                );
            }
            at = end;
        }
        panic!(
            "nothing matching {needle:?} carries an {event} listener:\n{}",
            self.hydrated
        )
    }

    fn dispatch(&mut self, event: &str, needle: &str, data: Box<dyn Any>) {
        let id = self.locate(event, needle);
        let payload: Rc<dyn Any> = Rc::new(PlatformEventData::new(data));
        {
            let _timers = install_once().enter();
            self.dom
                .runtime()
                .handle_event(event, Event::new(payload, true), id);
            self.dom.render_immediate(&mut NoOpMutations);
        }
        self.reread();
    }

    /// Tap the first control whose opening tag contains `needle`.
    pub(crate) fn press(&mut self, needle: &str) {
        self.dispatch("click", needle, Box::new(SerializedMouseData::default()));
    }

    /// Type into the first field whose opening tag contains `needle`, exactly
    /// as a `WebView` reports it: the field's whole new value.
    pub(crate) fn type_into(&mut self, needle: &str, value: &str) {
        self.dispatch(
            "input",
            needle,
            Box::new(SerializedFormData::new(value.to_owned(), Vec::new())),
        );
    }

    /// Press Enter in the first field whose opening tag contains `needle`.
    pub(crate) fn enter(&mut self, needle: &str) {
        self.dispatch(
            "keydown",
            needle,
            Box::new(SerializedKeyboardData::new(
                Key::Enter,
                Code::Enter,
                Location::Standard,
                false,
                Modifiers::empty(),
                false,
            )),
        );
    }
}
