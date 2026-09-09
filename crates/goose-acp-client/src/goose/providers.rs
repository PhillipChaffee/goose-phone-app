//! What model the NEXT conversation will run on, asked without one:
//! `_goose/unstable/providers/list` plus the two global config keys that pick
//! one entry out of it.
//!
//! # Why this file exists
//!
//! Every other route to a model list in this crate needs a session.
//! `session/new` types `configOptions` on its reply, `session/load` carries
//! the same array, and `session/set_config_option` hands it back — so a client
//! that has not opened a conversation has never been told what the agent
//! offers. Measured against `goose 1.46.0` on a real tailnet: a cold launch
//! left the chat home composer with a host chip and an extension count and no
//! model at all, which is the one thing that screen exists to let a reader
//! choose before starting the fifty-first conversation (#280). Nothing was
//! swallowing the answer; nothing had asked for it.
//!
//! # What it reconstructs, and why that is not a guess
//!
//! goose builds `session/new`'s `model` option out of exactly two things: the
//! global `GOOSE_MODEL` config value, and the model catalogue of the provider
//! named by `GOOSE_PROVIDER`. Both are readable with no session. Measured
//! against the same server on the same connection, the option
//! [`AcpClient::default_model_option`] assembles is the option `session/load`
//! returned: the same `currentValue`, and the same 168 choices in the same
//! order with the same names. That equality is not an implementation detail
//! this file hopes for — `the_reconstruction_equals_what_a_session_carries`
//! below holds the two shapes together against a captured pair of real
//! responses, which is why `model_option` is a free function and not four
//! lines inside an `async fn` no test can reach without a socket.
//!
//! # `config/read`, which this crate previously refused to wrap
//!
//! `goose/extensions.rs` says, and meant: *"there is deliberately no
//! `config/read` wrapper in this crate, because `config/read` on a secret
//! returns a clear prefix."* That refusal was about a wrapper taking a KEY —
//! which is a function that will read whatever a caller names, including
//! `OPENAI_API_KEY`. It is not an argument against reading `GOOSE_MODEL`.
//!
//! So the wrapper here does not take a key. [`GlobalKey`] has two variants and
//! no constructor from a string, so the set of keys reachable through this
//! crate is closed at compile time and neither member of it is a credential.
//! A future third key is a deliberate edit to this enum with this paragraph in
//! front of it, which is the whole difference between a narrow read and a
//! general one.
//!
//! `providers/list` is on the same side of that line for a different reason:
//! it returns the NAMES of each provider's config keys and a `secret: true`
//! flag beside them, never a value. Those names land in [`ProviderEntry`]'s
//! `extra`, and nothing on the path to a chip keeps one:
//! [`AcpClient::default_model_option`] drops every entry before it returns.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::LIST_TIMEOUT;
use crate::client::AcpClient;
use crate::error::AcpError;
use crate::types::{ConfigChoice, ConfigOption};

/// Every provider goose knows about, configured or not.
const PROVIDERS_LIST: &str = "_goose/unstable/providers/list";

/// One global config value, by name. See the module doc for why the name is
/// not a parameter of this crate's public API.
const CONFIG_READ: &str = "_goose/unstable/config/read";

/// The id a `model` config option carries, in goose's spelling.
///
/// Not a public constant, because the screens that look for it already spell
/// it themselves and a second spelling is not a fix for a first one; it is
/// here so this file, which MINTS an option rather than parsing one, cannot
/// mint it under a name the readers do not check for.
const MODEL_ID: &str = "model";

/// The two global config keys this crate will read, and the whole set of them.
///
/// A closed enum rather than a `&str` parameter on purpose: see the module
/// doc. Neither of these is a secret, and no third key can be asked for
/// without editing this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalKey {
    /// Which provider a session created right now would run on.
    Provider,
    /// Which of that provider's models it would run on.
    Model,
}

impl GlobalKey {
    /// The key's spelling on the wire.
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Provider => "GOOSE_PROVIDER",
            Self::Model => "GOOSE_MODEL",
        }
    }
}

/// The reply to `providers/list`.
///
/// Per this module's casing rule the field carries no attribute: `entries` is
/// one lowercase word and the Rust name IS the wire name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderListResponse {
    /// One [`ProviderEntry`] per provider the server knows, configured or
    /// not, in the order it lists them.
    pub entries: Vec<ProviderEntry>,
    /// Serde catch-all: reply keys this struct does not model land here and
    /// survive a round trip.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One provider, with the models it offers.
///
/// Mixed casing, so per the module rule each camelCase field carries its own
/// rename and `configured` and `models` carry none. `configKeys`,
/// `description`, `setupSteps`, `providerType`, `stale`, `refreshing` and
/// `supportsRefresh` are all real fields on the wire and none of them is
/// modelled: this crate asks this question to fill one chip, and a field
/// nothing reads is a field that goes stale without anyone noticing. They ride
/// in `extra`, where the round-trip check can still see them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderEntry {
    /// The provider's id, the value a `GOOSE_PROVIDER` config value names.
    /// The wire spells it `providerId`.
    #[serde(rename = "providerId")]
    pub provider_id: String,
    /// The provider's human-readable name, distinct from
    /// [`ProviderEntry::provider_id`]. The wire spells it `providerName`.
    #[serde(rename = "providerName")]
    pub provider_name: String,
    /// Whether this provider has the credentials it needs. Fourteen of the
    /// seventy-six on the server this was measured against answered true, so
    /// it is a real filter and not a formality — but it is NOT how the current
    /// provider is found. Nothing in this reply says which one is in use;
    /// [`GlobalKey::Provider`] does.
    pub configured: bool,
    /// The provider's own pick, which is not the user's. On the measured
    /// server `together`'s default was `Hcompany/Holo3-35B-A3B` while
    /// `GOOSE_MODEL` was `deepseek-ai/DeepSeek-V4-Pro-0813`, so reading this
    /// instead of the config key would have put a model on screen that no
    /// session was ever going to run on.
    #[serde(rename = "defaultModel")]
    pub default_model: Option<String>,
    /// The provider's model catalogue, in the order the server lists it —
    /// what [`AcpClient::default_model_option`] turns into the `model`
    /// option's choices.
    pub models: Vec<ProviderModel>,
    /// Serde catch-all: keys the server sent on the provider that this
    /// struct does not model land here and survive a round trip.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One model in a provider's catalogue.
///
/// `contextLimit`, `family`, `reasoning` and `recommended` are on the wire and
/// deliberately unmodelled, for [`ProviderEntry`]'s reason. `contextLimit` is
/// the tempting one — the composer's context chip waits for a `usage_update`
/// and this reply knows the window before the first turn — and it stays out of
/// this change because a chip that says a number is a decision about what the
/// screen claims, not a decode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderModel {
    /// The model's id — the value a `GOOSE_MODEL` config value names, and
    /// what a choice with no usable `name` shows instead.
    pub id: String,
    /// The catalogue's own label. Often the id verbatim — every one of
    /// `together`'s 168 is — which is why the empty case falls back to the id
    /// rather than to a blank.
    pub name: Option<String>,
    /// Serde catch-all: catalogue keys the server sent that this struct
    /// does not model land here and survive a round trip.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ProviderModel {
    /// The label to show, never empty.
    fn label(&self) -> &str {
        self.name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.id)
    }
}

/// Assemble the `model` option a session would carry, out of the two things
/// goose assembles it from.
///
/// A free function so the equality this module claims — that the result is the
/// option `session/load` returns — is a pure comparison of two captured
/// responses rather than something only a live server could check.
fn model_option(current: String, catalogue: &[ProviderModel]) -> ConfigOption {
    ConfigOption {
        config_id: MODEL_ID.to_owned(),
        name: "Model".to_owned(),
        // goose sends none for `model`; only `thinking_effort` carries one.
        description: None,
        category: Some(MODEL_ID.to_owned()),
        kind: Some("select".to_owned()),
        current_value: Some(current),
        options: catalogue
            .iter()
            .map(|model| ConfigChoice {
                value: model.id.clone(),
                name: model.label().to_owned(),
                // goose describes its MODES and nothing else, so a model
                // choice carrying one here would be a sentence this client
                // wrote and attributed to the agent.
                description: None,
            })
            .collect(),
    }
}

impl AcpClient {
    /// Read one of the two global config values.
    ///
    /// `Ok(None)` covers both "goose has no value for this" and "goose has an
    /// empty string", which mean the same thing to every caller here and are
    /// two different shapes on the wire.
    ///
    /// # Errors
    ///
    /// [`AcpError::Unsupported`] if the server has no `config/read` — every
    /// caller in this crate treats that as "not told" rather than as a
    /// failure. Otherwise [`AcpError::Rpc`], [`AcpError::Timeout`] after 30 s,
    /// or [`AcpError::Closed`].
    pub async fn read_global(&self, key: GlobalKey) -> Result<Option<String>, AcpError> {
        let raw = self
            .goose_request(CONFIG_READ, json!({"key": key.wire()}), LIST_TIMEOUT)
            .await?;
        Ok(raw
            .get("value")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned))
    }

    /// Every provider goose knows about, in the order it lists them.
    ///
    /// # Errors
    ///
    /// [`AcpError::Unsupported`] if the server has no `providers/list`,
    /// [`AcpError::Transport`] if the reply is not a
    /// [`ProviderListResponse`], or otherwise as [`Self::read_global`].
    pub async fn providers(&self) -> Result<Vec<ProviderEntry>, AcpError> {
        let raw = self
            .goose_request(PROVIDERS_LIST, json!({}), LIST_TIMEOUT)
            .await?;
        let parsed: ProviderListResponse =
            serde_json::from_value(raw).map_err(|e| AcpError::Transport(e.to_string()))?;
        Ok(parsed.entries)
    }

    /// The `model` config option a session created right now would carry.
    ///
    /// `Ok(None)` means the server has no model configured at all, which is a
    /// goose nobody has run `goose configure` against. That is the one state
    /// in which there is genuinely nothing to say, and it is the only one this
    /// returns nothing for: a server that answers the model but not the
    /// catalogue still yields an option, with no choices in it, because
    /// [`ConfigOption::is_adjustable`] already knows that a set the reader
    /// cannot choose within is a fact rather than a control.
    ///
    /// # Errors
    ///
    /// As [`Self::read_global`] — the catalogue's own failure is absorbed, see
    /// the body.
    pub async fn default_model_option(&self) -> Result<Option<ConfigOption>, AcpError> {
        let Some(current) = self.read_global(GlobalKey::Model).await? else {
            return Ok(None);
        };
        let provider = self.read_global(GlobalKey::Provider).await?;
        // A catalogue this server will not hand over costs the reader the
        // MENU, not the FACT. `providers/list` is 134 KB and the newest of the
        // three calls; the model name is 30 bytes and answered by a key goose
        // has had for as long as it has had providers. Failing the whole call
        // on the big one would trade a chip that says what the next chat runs
        // on for no chip at all, which is the state #280 is about.
        let catalogue = match provider {
            Some(provider) => self.provider_models(&provider).await.unwrap_or_default(),
            None => Vec::new(),
        };
        Ok(Some(model_option(current, &catalogue)))
    }

    /// One provider's catalogue.
    ///
    /// A provider id the list does not carry yields an empty set rather than
    /// an error: goose will happily hold a `GOOSE_PROVIDER` for a provider it
    /// no longer ships, and that is a menu this app cannot offer, not a
    /// connection that has gone wrong.
    async fn provider_models(&self, provider_id: &str) -> Result<Vec<ProviderModel>, AcpError> {
        Ok(self
            .providers()
            .await?
            .into_iter()
            .find(|entry| entry.provider_id == provider_id)
            .map(|entry| entry.models)
            .unwrap_or_default())
    }
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "test assertions: a failing unwrap is the failing check"
)]
mod tests {
    use super::*;
    use crate::assert_round_trip;

    /// One `providers/list` entry, VERBATIM from `goose 1.46.0` — every key
    /// the server sent, in the order it sent them, with only the 168-long
    /// `models` array cut to two. Complete rather than minimal, because
    /// [`assert_round_trip`] is what catches a field this crate spells wrong
    /// and it can only see the keys the fixture carries.
    fn together() -> Value {
        json!({
          "category": "model",
          "configKeys": [
            {"default": null, "deviceCodeFlow": false, "name": "OPENAI_CUSTOM_HEADERS",
             "oauthFlow": false, "primary": false, "required": false, "secret": true},
            {"default": "600", "deviceCodeFlow": false, "name": "OPENAI_TIMEOUT",
             "oauthFlow": false, "primary": false, "required": false, "secret": false}
          ],
          "configured": true,
          "defaultModel": "Hcompany/Holo3-35B-A3B",
          "description": "Custom Together AI — ZDR, sensitive-safe provider",
          "models": [
            {"contextLimit": 262_144, "id": "Hcompany/Holo3-35B-A3B",
             "name": "Hcompany/Holo3-35B-A3B", "recommended": false},
            {"contextLimit": 1_048_576, "id": "MiniMaxAI/MiniMax-M1-40k",
             "name": "MiniMaxAI/MiniMax-M1-40k", "recommended": false}
          ],
          "providerId": "together",
          "providerName": "Together AI — ZDR, sensitive-safe",
          "providerType": "Custom",
          "refreshing": false,
          "setupSteps": [],
          "stale": false,
          "supportsRefresh": false
        })
    }

    #[test]
    fn a_provider_entry_round_trips_every_key_the_server_sent() {
        let entry: ProviderEntry = assert_round_trip(&together());
        assert_eq!(entry.provider_id, "together");
        assert_eq!(entry.provider_name, "Together AI — ZDR, sensitive-safe");
        assert!(entry.configured);
        assert_eq!(
            entry.default_model.as_deref(),
            Some("Hcompany/Holo3-35B-A3B")
        );
        assert_eq!(entry.models.len(), 2);
        // The seven fields this crate does not model are still here.
        assert!(entry.extra.contains_key("configKeys"));
        assert!(entry.extra.contains_key("supportsRefresh"));
        assert!(entry.models[0].extra.contains_key("contextLimit"));
    }

    #[test]
    fn the_whole_reply_round_trips_too() {
        let reply = json!({"entries": [together()]});
        let parsed: ProviderListResponse = assert_round_trip(&reply);
        assert_eq!(parsed.entries.len(), 1);
    }

    /// The two keys, spelled once. goose reads `GOOSE_MODEL` and
    /// `GOOSE_PROVIDER` from its own config; a typo here is a `{"value":
    /// null}` and a home screen back where it started.
    #[test]
    fn the_two_global_keys_keep_their_wire_spelling() {
        assert_eq!(GlobalKey::Provider.wire(), "GOOSE_PROVIDER");
        assert_eq!(GlobalKey::Model.wire(), "GOOSE_MODEL");
    }

    /// A catalogue name is often the id verbatim, and an absent or blank one
    /// must not become a blank row in the picker.
    #[test]
    fn a_model_with_no_name_is_labelled_by_its_id() {
        let named: ProviderModel =
            serde_json::from_value(json!({"id": "glm-5", "name": "GLM 5"})).unwrap();
        assert_eq!(named.label(), "GLM 5");
        let bare: ProviderModel = serde_json::from_value(json!({"id": "glm-5"})).unwrap();
        assert_eq!(bare.label(), "glm-5");
        let blank: ProviderModel =
            serde_json::from_value(json!({"id": "glm-5", "name": "   "})).unwrap();
        assert_eq!(blank.label(), "glm-5");
    }

    /// THE CLAIM THIS WHOLE FILE RESTS ON, held against two captures of the
    /// same server taken on the same connection.
    ///
    /// Left: the `model` entry `session/load` returned for session
    /// `20260907_17`, verbatim, `id`-keyed the way goose 1.46.0 keys it, its
    /// 168 options cut to the first two. Right: what this module builds from
    /// `GOOSE_MODEL` and `together`'s catalogue, cut the same way. If goose
    /// ever starts building a session's model option out of something else,
    /// this is the test that says so — and it says so without a server,
    /// because both sides are recorded responses.
    #[test]
    fn the_reconstruction_equals_what_a_session_carries() {
        let from_session: ConfigOption = serde_json::from_value(json!({
            "category": "model",
            "currentValue": "deepseek-ai/DeepSeek-V4-Pro-0813",
            "id": "model",
            "name": "Model",
            "options": [
                {"name": "Hcompany/Holo3-35B-A3B", "value": "Hcompany/Holo3-35B-A3B"},
                {"name": "MiniMaxAI/MiniMax-M1-40k", "value": "MiniMaxAI/MiniMax-M1-40k"}
            ],
            "type": "select"
        }))
        .unwrap();

        let entry: ProviderEntry = serde_json::from_value(together()).unwrap();
        let built = model_option("deepseek-ai/DeepSeek-V4-Pro-0813".to_owned(), &entry.models);

        assert_eq!(built, from_session);
        // And the three things the composer actually asks of it.
        assert_eq!(built.config_id, MODEL_ID);
        assert!(built.is_adjustable(), "two choices is a control");
        assert_eq!(
            built.current_label(),
            Some("deepseek-ai/DeepSeek-V4-Pro-0813"),
            "a current value the catalogue does not name still labels itself"
        );
    }

    /// A catalogue with one model in it is a fact, not a menu — the same rule
    /// `thinking_effort` is read by, applied to the same struct.
    #[test]
    fn a_single_model_provider_is_a_fact_and_not_a_control() {
        let entry: ProviderModel =
            serde_json::from_value(json!({"id": "gpt-5.2-codex", "name": "gpt-5.2-codex"}))
                .unwrap();
        let built = model_option("gpt-5.2-codex".to_owned(), std::slice::from_ref(&entry));
        assert!(!built.is_adjustable());
        assert_eq!(built.current_label(), Some("gpt-5.2-codex"));
    }

    /// `Feature::of_method` already classifies both of these — `error.rs` has
    /// carried `("_goose/unstable/providers/list", Feature::Other)` since
    /// before anything called it — so an old goose answers `-32601` and the
    /// caller sees `Unsupported` rather than a raw code.
    #[test]
    fn both_methods_live_in_gooses_own_namespace() {
        for method in [PROVIDERS_LIST, CONFIG_READ] {
            assert!(method.starts_with(super::super::GOOSE_NAMESPACE));
        }
    }
}
