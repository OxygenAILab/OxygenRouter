//! JavaScript plugin host: load a plugin, call its hooks out of process-safe
//! single-threaded state, and bound how long a hook may run.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover
//!
//! # Why this shape
//!
//! The reference implements its plugin system with `grafana/sobek`, a pure-Go
//! JavaScript engine (`pkg/jsplugin/engine.go:13`), so an operator can drop a
//! `.js` file in and the gateway picks it up with no toolchain. The Rust
//! equivalent is `boa_engine`, also pure Rust, so the same single-binary
//! guarantee holds and no C compiler is needed to build the project.
//!
//! `boa_engine::Context` is neither `Send` nor `Sync`. Instead of hiding that
//! behind a mutex and hoping, the engine lives on one dedicated thread and every
//! call is a message. That gives three things for free and on purpose:
//!
//! * a plugin that blocks its thread blocks only plugins, never the gateway;
//! * calls are serialised, so two hooks cannot interleave inside one engine;
//! * a timeout is enforced by abandoning the work rather than by trusting the
//!   script to yield.
//!
//! A plugin's script is untrusted code. It is not sandboxed against its own
//! engine — a plugin can loop forever, which is why the timeout exists — but it
//! has no access to the filesystem, the network or the process: the only values
//! it sees are the ones a hook call passes in, and the only thing it can return
//! is JSON.

use std::sync::mpsc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The manifest API version this host understands.
///
/// Mirrors the reference's `APIVersion1` (`pkg/jsplugin/registry.go:29`), so a
/// manifest written for it is accepted here.
pub const API_VERSION_1: u32 = 1;

/// Default ceiling on one hook call, matching the reference's
/// `DefaultCallTimeout` (`pkg/jsplugin/engine.go:18`).
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Why a plugin could not be loaded or a hook could not run.
#[derive(Debug, Clone, thiserror::Error)]
pub enum PluginError {
    #[error("plugin manifest is not valid JSON: {0}")]
    BadManifest(String),
    #[error("plugin declares apiVersion {found}, but this host speaks {expected}")]
    UnsupportedApiVersion { found: u32, expected: u32 },
    #[error("plugin key {0:?} is not a valid identifier")]
    BadKey(String),
    #[error("plugin source did not run: {0}")]
    Load(String),
    #[error("plugin {key:?} has no hook {hook:?}")]
    NoSuchHook { key: String, hook: String },
    #[error("plugin call did not finish within {0:?}")]
    Timeout(Duration),
    #[error("plugin {key:?} hook {hook:?} threw: {message}")]
    Hook {
        key: String,
        hook: String,
        message: String,
    },
    #[error("plugin host stopped")]
    HostStopped,
}

/// One routing mode a plugin declares.
///
/// The reference groups hooks under named "modes" (`pkg/jsplugin/registry.go`),
/// so a mode says which platform it serves and which hook it implements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMode {
    pub name: String,
    /// Which hook this mode implements, e.g. `convertRequest`.
    pub hook: String,
    /// Protocols the mode serves, when it declares any.
    #[serde(default)]
    pub supports: Vec<String>,
}

/// A plugin's declared metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    #[serde(rename = "apiVersion")]
    pub api_version: u32,
    /// Canonical identifier; the reference restricts it to
    /// `^[a-z0-9][a-z0-9_-]*$` and at most 30 characters.
    pub key: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub modes: Vec<PluginMode>,
    /// Protocols this plugin claims to serve. Validated against the host's table
    /// and against what the source actually exports.
    #[serde(default)]
    pub protocols: Vec<ProtocolClaim>,
}

impl PluginManifest {
    /// Validate the fields the host relies on.
    fn validate(&self) -> Result<(), PluginError> {
        if self.api_version != API_VERSION_1 {
            return Err(PluginError::UnsupportedApiVersion {
                found: self.api_version,
                expected: API_VERSION_1,
            });
        }
        if !valid_key(&self.key) {
            return Err(PluginError::BadKey(self.key.clone()));
        }
        Ok(())
    }

    /// True when the manifest declares a mode implementing `hook`.
    pub fn implements(&self, hook: &str) -> bool {
        self.modes.iter().any(|mode| mode.hook == hook)
    }
}

/// The reference's `pluginKeyPattern`, transcribed.
pub fn valid_key(key: &str) -> bool {
    if key.is_empty() || key.len() > 30 {
        return false;
    }
    let mut chars = key.chars();
    let first = chars.next().expect("checked non-empty");
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// A command sent to the engine thread.
enum Call {
    Load {
        source: String,
        reply: mpsc::Sender<Result<PluginManifest, PluginError>>,
    },
    Hook {
        key: String,
        hook: String,
        /// JSON-encoded argument list.
        input: String,
        reply: mpsc::Sender<Result<String, PluginError>>,
    },
}

/// Handle to the engine thread.
///
/// Cheap to clone; every clone shares the one engine.
#[derive(Clone)]
pub struct PluginHost {
    tx: mpsc::Sender<Call>,
}

impl PluginHost {
    /// Start a host with its own engine thread.
    pub fn start() -> Self {
        let (tx, rx) = mpsc::channel::<Call>();
        std::thread::Builder::new()
            .name("oxygenrouter-plugin".to_string())
            .spawn(move || engine_loop(rx))
            .expect("plugin thread");
        Self { tx }
    }

    /// Load a plugin from its source and return the manifest it declared.
    ///
    /// The source is expected to end by calling a global `register(manifest)`,
    /// which is how the reference's launcher contract works; if it never does,
    /// the plugin is rejected rather than silently contributing nothing.
    pub async fn load(&self, source: String, timeout: Duration) -> Result<PluginManifest, PluginError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Call::Load {
                source,
                reply: reply_tx,
            })
            .map_err(|_| PluginError::HostStopped)?;
        await_reply(reply_rx, timeout).await
    }

    /// Call one hook, passing and receiving JSON.
    pub async fn call_hook(
        &self,
        key: &str,
        hook: &str,
        input: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, PluginError> {
        self.call_with_args(key, hook, &[input], timeout).await
    }

    /// `protocols.<protocol>.decodeRequest(ctx)` — the client request, turned
    /// into the request the upstream should receive.
    ///
    /// The argument is the *protocol* name (`openai_responses`), which is the key
    /// the plugin exports under, not the plugin's own key. A caller holding a
    /// plugin key resolves it through that plugin's protocol claims.
    pub async fn decode_request(
        &self,
        protocol: &str,
        ctx: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, PluginError> {
        self.call_with_args(protocol, "decodeRequest", &[ctx], timeout).await
    }

    /// `protocols.<protocol>.renderFinal(ctx, task)` — the upstream's result,
    /// turned into what the client asked for.
    ///
    /// Two arguments because the reference passes both: the original request
    /// context, so a renderer can echo what the client sent, and the task the
    /// upstream produced.
    pub async fn render_final(
        &self,
        protocol: &str,
        ctx: serde_json::Value,
        task: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, PluginError> {
        self.call_with_args(protocol, "renderFinal", &[ctx, task], timeout).await
    }

    /// Send one call and decode the JSON it returned.
    async fn call_with_args(
        &self,
        key: &str,
        hook: &str,
        args: &[serde_json::Value],
        timeout: Duration,
    ) -> Result<serde_json::Value, PluginError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Call::Hook {
                key: key.to_string(),
                hook: hook.to_string(),
                input: serde_json::to_string(args).unwrap_or_else(|_| "[]".to_string()),
                reply: reply_tx,
            })
            .map_err(|_| PluginError::HostStopped)?;
        let raw = await_reply(reply_rx, timeout).await?;
        serde_json::from_str(&raw).map_err(|error| PluginError::Hook {
            key: key.to_string(),
            hook: hook.to_string(),
            message: format!("hook returned a value that is not JSON: {error}"),
        })
    }
}

/// Wait for the engine thread's answer, giving up after `timeout`.
///
/// The engine thread is not interrupted; a hook that never returns keeps its
/// thread busy and later calls queue behind it. That is a deliberate trade: the
/// alternative is letting a plugin block the caller indefinitely, and a plugin
/// busy-looping is a bug its author must fix either way.
async fn await_reply<T: Send + 'static>(
    reply: mpsc::Receiver<Result<T, PluginError>>,
    timeout: Duration,
) -> Result<T, PluginError> {
    // `recv_timeout` would block the async runtime; a blocking thread is used
    // instead so the caller stays cooperative.
    let handle = tokio::task::spawn_blocking(move || {
        reply.recv_timeout(timeout).map_err(|_| PluginError::Timeout(timeout))
    });
    match handle.await {
        Ok(Ok(inner)) => inner,
        Ok(Err(error)) => Err(error),
        Err(_) => Err(PluginError::HostStopped),
    }
}

fn engine_loop(rx: mpsc::Receiver<Call>) {
    let mut engine = Engine::new();
    while let Ok(call) = rx.recv() {
        match call {
            Call::Load { source, reply } => {
                let _ = reply.send(engine.load(&source));
            }
            Call::Hook {
                key,
                hook,
                input,
                reply,
            } => {
                let args: Vec<serde_json::Value> =
                    serde_json::from_str(&input).unwrap_or_else(|_| vec![serde_json::Value::Null]);
                let _ = reply.send(engine.call_member(&key, &hook, &args));
            }
        }
    }
}

/// The engine thread's private state. Never leaves this thread.
struct Engine {
    context: boa_engine::Context,
    /// Keys of plugins that loaded successfully, so a hook call for an unknown
    /// plugin is refused rather than silently doing nothing.
    loaded: std::collections::HashSet<String>,
}

impl Engine {
    fn new() -> Self {
        Self {
            context: boa_engine::Context::default(),
            loaded: std::collections::HashSet::new(),
        }
    }

    /// The launcher prelude, evaluated before every plugin.
    ///
    /// `register` is defined in the script rather than as a host callback: the
    /// host callback would have to capture shared state across Boa's garbage
    /// collector, and a two-line prelude does the same job with nothing to trace.
    /// The reset to `null` matters — without it a plugin that forgets to register
    /// would inherit the previous plugin's manifest.
    const PRELUDE: &'static str = "\
        var __ORM_MANIFEST = null;\n\
        function register(manifest) { __ORM_MANIFEST = manifest; }\n";

    /// Run a plugin's source and return the manifest it registered.
    fn load(&mut self, source: &str) -> Result<PluginManifest, PluginError> {
        use boa_engine::Source;

        self.context
            .eval(Source::from_bytes(Self::PRELUDE))
            .map_err(|error| PluginError::Load(describe(&error)))?;
        self.context
            .eval(Source::from_bytes(source))
            .map_err(|error| PluginError::Load(describe(&error)))?;

        // Read the manifest back through JSON so the host holds a plain value
        // rather than a live JS object it could accidentally keep a handle to.
        let raw = self
            .context
            .eval(Source::from_bytes("JSON.stringify(__ORM_MANIFEST)"))
            .map_err(|error| PluginError::Load(describe(&error)))?
            .to_string(&mut self.context)
            .map_err(|error| PluginError::Load(describe(&error)))?
            .to_std_string_escaped();
        if raw == "undefined" || raw == "null" {
            return Err(PluginError::Load(
                "plugin did not call register(manifest)".to_string(),
            ));
        }
        let manifest: PluginManifest = serde_json::from_str(&raw)
            .map_err(|error| PluginError::BadManifest(error.to_string()))?;
        manifest.validate()?;

        // A manifest is a claim; an export is a fact. The host asks the engine
        // which protocol members the source actually defined and refuses the
        // plugin when the two disagree, so a manifest cannot promise a hook that
        // does not exist and only fail at request time.
        let exported = self.exported_protocol_members()?;
        let problems = validate_protocol_claims(&manifest.protocols, &manifest.key, &exported);
        if !problems.is_empty() {
            return Err(PluginError::Load(problems.join("; ")));
        }

        self.loaded.insert(manifest.key.clone());
        // The protocol members are exported under the protocol's name, not the
        // plugin's (`pkg/jsplugin/protocol_supports_test.go:13` exports
        // `protocols.openai_responses.*`), so both spellings are addressable.
        for claim in &manifest.protocols {
            self.loaded.insert(claim.name.clone());
        }
        Ok(manifest)
    }

    /// Every `protocols.<protocol>.<member>` the loaded source defines as a
    /// function, discovered from the host table rather than from the manifest.
    fn exported_protocol_members(&mut self) -> Result<Vec<String>, PluginError> {
        use boa_engine::Source;

        let mut probe = String::from("(function () { var out = [];");
        for protocol in HOST_PROTOCOLS {
            let mut members: Vec<&str> = Vec::new();
            for operation in protocol.operations {
                members.extend_from_slice(operation.required_members);
                members.extend_from_slice(operation.required_driver_hooks);
                for mode in operation.modes {
                    members.push(mode.hook);
                }
            }
            members.sort_unstable();
            members.dedup();
            for member in members {
                probe.push_str(&format!(
                    "try {{ if (typeof protocols !== 'undefined' && protocols[{p:?}] && typeof protocols[{p:?}][{m:?}] === 'function') out.push({full:?}); }} catch (e) {{}}\n",
                    p = protocol.name,
                    m = member,
                    full = format!("{}.{}", protocol.name, member),
                ));
            }
        }
        probe.push_str("return JSON.stringify(out); })()");

        let raw = self
            .context
            .eval(Source::from_bytes(probe.as_bytes()))
            .map_err(|error| PluginError::Load(describe(&error)))?
            .to_string(&mut self.context)
            .map_err(|error| PluginError::Load(describe(&error)))?
            .to_std_string_escaped();
        let names: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
        // The probe reports `protocol.member`; validation compares member names,
        // because a hook name is unique within the host's table.
        Ok(names
            .into_iter()
            .filter_map(|full| full.split_once('.').map(|(_, member)| member.to_string()))
            .collect())
    }

    /// Call `protocols.<key>.<member>` with JSON arguments, returning JSON.
    ///
    /// The member is reached through the plugin's `protocols` export rather than
    /// a name the host made up, so the call site is the same shape the reference
    /// uses and a plugin written for it works here unchanged.
    fn call_member(
        &mut self,
        key: &str,
        member: &str,
        args: &[serde_json::Value],
    ) -> Result<String, PluginError> {
        use boa_engine::Source;

        if !self.loaded.contains(key) {
            return Err(PluginError::NoSuchHook {
                key: key.to_string(),
                hook: member.to_string(),
            });
        }
        let argv: Vec<String> = args
            .iter()
            .map(|a| serde_json::to_string(a).unwrap_or_else(|_| "null".to_string()))
            .collect();
        let script = format!(
            "(function () {{\n\
             var __plugin = (typeof protocols !== 'undefined' && protocols[{key_json}]) ||\n\
                            (typeof plugins !== 'undefined' && plugins[{key_json}]) ||\n\
                            (typeof module !== 'undefined' && module.exports && module.exports[{key_json}]) ||\n\
                            (typeof exports !== 'undefined' && exports[{key_json}]);\n\
             if (!__plugin) throw new Error('plugin ' + {key_json} + ' did not export an object');\n\
             var __fn = __plugin[{member_json}];\n\
             if (typeof __fn !== 'function') throw new Error('plugin ' + {key_json} + ' has no member ' + {member_json});\n\
             return JSON.stringify(__fn.apply(null, [{argv}]));\n\
             }})()",
            key_json = serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string()),
            member_json = serde_json::to_string(member).unwrap_or_else(|_| "\"\"".to_string()),
            argv = argv.join(", "),
        );

        let value = self
            .context
            .eval(Source::from_bytes(script.as_bytes()))
            .map_err(|error| PluginError::Hook {
                key: key.to_string(),
                hook: member.to_string(),
                message: describe(&error),
            })?;
        value
            .to_string(&mut self.context)
            .map_err(|error| PluginError::Hook {
                key: key.to_string(),
                hook: member.to_string(),
                message: describe(&error),
            })
            .map(|s| s.to_std_string_escaped())
    }
}

/// Turn an engine error into a short, single-line message.
///
/// A plugin's stack trace is not the caller's business, and a hook error may be
/// surfaced to an API client, so the text is flattened and truncated.
fn describe(error: &boa_engine::JsError) -> String {
    let text = error.to_string();
    let mut out = String::new();
    for ch in text.chars() {
        if out.len() >= 512 {
            break;
        }
        out.push(if ch.is_control() { ' ' } else { ch });
    }
    if out.is_empty() {
        "plugin hook failed".to_string()
    } else {
        out
    }
}

// ── Host protocols ─────────────────────────────────────────────────────────────

/// One client request form an operation accepts, and the hook implementing it.
///
/// Ported from the reference's `ProtocolMode` (`pkg/jsplugin/routing.go:57`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolMode {
    /// The request form's name, e.g. `stream` or `sync`.
    pub name: &'static str,
    /// The hook a plugin must export to serve it.
    pub hook: &'static str,
}

/// One endpoint an operation serves.
///
/// Ported from `HostProtocolOperation` (`pkg/jsplugin/routing.go:71`). The
/// operation is what a plugin is contracted to implement: to serve
/// `POST /v1/responses` a plugin must export the members listed here plus the
/// hook for whichever request forms it claims.
#[derive(Debug, Clone, Copy)]
pub struct HostOperation {
    pub name: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    /// Members every plugin serving this operation must export, whatever forms
    /// it claims.
    pub required_members: &'static [&'static str],
    /// Request forms, each naming the hook that implements it.
    pub modes: &'static [ProtocolMode],
    /// Hooks required regardless of the forms claimed.
    pub required_driver_hooks: &'static [&'static str],
}

/// A client-facing protocol the host knows how to serve from a plugin.
#[derive(Debug, Clone, Copy)]
pub struct HostProtocol {
    pub name: &'static str,
    pub operations: &'static [HostOperation],
}

/// The protocols a plugin may claim.
///
/// A curated list rather than free naming: each entry is a promise about the
/// hook contract, and the host can only keep a promise it knows. The first entry
/// is the Responses API, which can only expose its full streaming semantics if a
/// plugin renders them itself.
pub const HOST_PROTOCOLS: &[HostProtocol] = &[HostProtocol {
    name: "openai_responses",
    operations: &[
        HostOperation {
            name: "create",
            method: "POST",
            path: "/v1/responses",
            required_members: &["decodeRequest"],
            modes: &[
                ProtocolMode {
                    name: "stream",
                    hook: "renderEvents",
                },
                ProtocolMode {
                    name: "sync",
                    hook: "renderFinal",
                },
                ProtocolMode {
                    name: "background",
                    hook: "renderFinal",
                },
            ],
            required_driver_hooks: &[],
        },
        HostOperation {
            name: "retrieve",
            method: "GET",
            path: "/v1/responses/:response_id",
            required_members: &[],
            modes: &[],
            required_driver_hooks: &[],
        },
    ],
}];

/// Look up a protocol by the name a manifest claims.
pub fn host_protocol(name: &str) -> Option<&'static HostProtocol> {
    HOST_PROTOCOLS.iter().find(|p| p.name == name)
}

/// What a manifest claims about one protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolClaim {
    pub name: String,
    /// Models this claim covers; empty means every model the plugin claims.
    #[serde(default)]
    pub models: Vec<String>,
    /// Which request forms the plugin serves. Empty means none are declared, and
    /// validation will say so rather than silently serving nothing.
    #[serde(default)]
    pub supports: Vec<String>,
}

/// Check a set of claims against the host's protocol table and the members the
/// plugin actually exported.
///
/// `exported` is the plugin's own list of `protocols.<name>.<member>` entries —
/// the host asks the engine for it rather than trusting the manifest, because a
/// manifest is a claim and an export is a fact. Each problem names the protocol
/// and the hook: "supports sync but does not export
/// protocols.openai_responses.renderFinal" tells an author exactly what to write,
/// where "invalid plugin" does not. The reference phrases its equivalents the
/// same way (`pkg/jsplugin/registry.go:464`).
pub fn validate_protocol_claims(
    claims: &[ProtocolClaim],
    key: &str,
    exported: &[String],
) -> Vec<String> {
    let mut problems = Vec::new();
    for claim in claims {
        let Some(protocol) = host_protocol(&claim.name) else {
            problems.push(format!(
                "plugin {key} claims protocol {:?}, which this host does not serve",
                claim.name
            ));
            continue;
        };
        if claim.supports.is_empty() {
            problems.push(format!(
                "plugin {key} protocol {:?} declares no supports; name the request forms it serves",
                claim.name
            ));
            continue;
        }
        for support in &claim.supports {
            let modes: Vec<&ProtocolMode> = protocol
                .operations
                .iter()
                .flat_map(|op| op.modes.iter())
                .filter(|mode| mode.name == support)
                .collect();
            if modes.is_empty() {
                problems.push(format!(
                    "plugin {key} protocol {:?} declares supports {:?}, which is not a request form of that protocol",
                    claim.name, support
                ));
                continue;
            }
            // The hook this form needs, and the members the operation it belongs
            // to needs whichever form was claimed.
            let mut required: Vec<&str> = Vec::new();
            let mut hooks: Vec<&str> = Vec::new();
            for operation in protocol.operations {
                if operation.modes.iter().any(|m| m.name == support) {
                    required.extend_from_slice(operation.required_members);
                    if let Some(mode) = operation.modes.iter().find(|m| m.name == support) {
                        hooks.push(mode.hook);
                    }
                    required.extend_from_slice(operation.required_driver_hooks);
                }
            }
            for member in required.into_iter().chain(hooks.into_iter()) {
                if !exported.iter().any(|e| e == member) {
                    problems.push(format!(
                        "plugin {key} protocol {:?} supports {:?} but does not export protocols.{}.{}; implement it or stop claiming {:?}",
                        claim.name, support, claim.name, member, support
                    ));
                }
            }
            // A hook exported for a form that was not claimed is not an error --
            // the reference allows it -- so nothing is reported for extra members.
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plugin in the shape the reference's launcher contract expects: it
    /// declares a manifest and exports the hooks the manifest names.
    const DEMO: &str = r#"
        register({
            apiVersion: 1,
            key: "demo",
            name: "Demo",
            version: "1.0.0",
            description: "uppercases the model name",
            modes: [{ name: "openai", hook: "convertRequest" }]
        });
        var plugins = {
            demo: {
                convertRequest: function (input) {
                    return { model: String(input.model).toUpperCase(), seen: input.tokens };
                }
            }
        };
    "#;

    /// A plugin serving the Responses API must export the members its claim
    /// implies. The host asks the engine what the source actually defines rather
    /// than trusting the manifest, so this is the check that stops a plugin
    /// promising a hook it never wrote.
    #[test]
    fn the_responses_protocol_requires_the_members_it_names() {
        // The reference's own minimal example for this protocol
        // (`pkg/jsplugin/protocol_supports_test.go:14`).
        const COMPLETE: &str = r#"
            register({apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                modes:[{name:"responses", hook:"renderEvents"}],
                protocols:[{name:"openai_responses", supports:["stream"]}]});
            var protocols = { openai_responses: {
                decodeRequest: function(ctx) { return ctx; },
                renderEvents: function() { return {events: [], state: null, done: false}; }
            } };
        "#;
        // Same manifest, but renderEvents is missing.
        const MISSING_HOOK: &str = r#"
            register({apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                protocols:[{name:"openai_responses", supports:["stream"]}]});
            var protocols = { openai_responses: {
                decodeRequest: function(ctx) { return ctx; }
            } };
        "#;
        // Claims sync, which needs renderFinal, and only wrote renderEvents --
        // the exact shape the reference reports.
        const WRONG_MODE: &str = r#"
            register({apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                protocols:[{name:"openai_responses", supports:["sync"]}]});
            var protocols = { openai_responses: {
                decodeRequest: function(ctx) { return ctx; },
                renderEvents: function() { return {}; }
            } };
        "#;
        const UNKNOWN_PROTOCOL: &str = r#"
            register({apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                protocols:[{name:"made_up_protocol", supports:["stream"]}]});
            var protocols = { made_up_protocol: { decodeRequest: function(){return {};} } };
        "#;
        const NO_SUPPORTS: &str = r#"
            register({apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                protocols:[{name:"openai_responses", supports:[]}]});
            var protocols = { openai_responses: { decodeRequest: function(){return {};} } };
        "#;

        let host = PluginHost::start();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            host.load(COMPLETE.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("the complete plugin must load");

            for (label, source, want) in [
                ("missing hook", MISSING_HOOK, "does not export"),
                ("wrong mode", WRONG_MODE, "renderFinal"),
                ("unknown protocol", UNKNOWN_PROTOCOL, "does not serve"),
                ("no supports", NO_SUPPORTS, "declares no supports"),
            ] {
                let error = host
                    .load(source.to_string(), DEFAULT_CALL_TIMEOUT)
                    .await
                    .expect_err(label);
                let text = error.to_string();
                assert!(text.contains(want), "{label}: expected {want:?} in {text:?}");
            }
        });
    }

    /// The host table is the contract, so it is worth pinning that the protocol
    /// the reference serves is present and shaped the same way.
    #[test]
    fn the_host_serves_the_responses_protocol() {
        let protocol = host_protocol("openai_responses").expect("known protocol");
        assert!(host_protocol("nope").is_none());

        let create = protocol
            .operations
            .iter()
            .find(|op| op.name == "create")
            .expect("create operation");
        assert_eq!(create.method, "POST");
        assert_eq!(create.path, "/v1/responses");
        assert_eq!(create.required_members, &["decodeRequest"]);
        // Streaming and non-streaming are different hooks, which is the whole
        // reason this protocol needs a plugin: only the plugin can emit the
        // event stream the Responses API promises.
        let stream = create.modes.iter().find(|m| m.name == "stream").unwrap();
        let sync = create.modes.iter().find(|m| m.name == "sync").unwrap();
        assert_eq!(stream.hook, "renderEvents");
        assert_eq!(sync.hook, "renderFinal");

        // Validation is order-independent and reports every missing member, so an
        // author fixing one hook is told about the rest in the same pass.
        let problems = validate_protocol_claims(
            &[ProtocolClaim {
                name: "openai_responses".into(),
                models: Vec::new(),
                supports: vec!["stream".into(), "sync".into()],
            }],
            "acme",
            &[],
        );
        // One problem per (claimed form, missing member): `decodeRequest` is
        // required by both forms, so it is reported twice. That is deliberate --
        // each message names the claim that failed, so an author who removes the
        // `sync` claim still sees the `stream` one -- and it matches how the
        // reference words its equivalents.
        assert_eq!(problems.len(), 4, "{problems:?}");
        for member in ["decodeRequest", "renderEvents", "renderFinal"] {
            assert!(
                problems.iter().any(|p| p.contains(member)),
                "{member} is missing from {problems:?}"
            );
        }
        // Nothing is reported for a member the plugin did export.
        assert!(!problems.iter().any(|p| p.contains("attributes")));

        // With everything exported there is nothing to report.
        let exported = vec![
            "decodeRequest".to_string(),
            "renderEvents".to_string(),
            "renderFinal".to_string(),
        ];
        assert!(validate_protocol_claims(
            &[ProtocolClaim {
                name: "openai_responses".into(),
                models: Vec::new(),
                supports: vec!["stream".into(), "sync".into()],
            }],
            "acme",
            &exported,
        )
        .is_empty());
    }

    /// The two members the routing path will call, with the argument arity the
    /// reference gives them: `decodeRequest(ctx)` and `renderFinal(ctx, task)`.
    #[test]
    fn the_protocol_members_are_callable_with_their_own_arity() {
        const BRIDGE: &str = r#"
            register({apiVersion:1, key:"bridge", name:"Bridge", version:"1.0.0",
                protocols:[{name:"openai_responses", supports:["sync"]}]});
            var protocols = { openai_responses: {
                decodeRequest: function (ctx) {
                    // Reads the client body and states the upstream request.
                    return { upstream: { model: ctx.body.model, input: ctx.body.input } };
                },
                renderFinal: function (ctx, task) {
                    // Both arguments reach the hook: the original context and the
                    // upstream's result.
                    return { id: task.id, echoed: ctx.body.model, output: task.output };
                }
            } };
        "#;
        let host = PluginHost::start();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            host.load(BRIDGE.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("load");

            let ctx = serde_json::json!({
                "path": "/v1/responses",
                "method": "POST",
                "body": { "model": "acme-large", "input": "hello" }
            });
            let upstream = host
                .decode_request("openai_responses", ctx.clone(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("decodeRequest");
            assert_eq!(upstream["upstream"]["model"], "acme-large");

            let task = serde_json::json!({ "id": "resp_1", "output": "hi there" });
            let rendered = host
                .render_final("openai_responses", ctx.clone(), task, DEFAULT_CALL_TIMEOUT)
                .await
                .expect("renderFinal");
            assert_eq!(rendered["id"], "resp_1");
            // Proves the second argument arrived; a one-argument call would have
            // thrown on `task.id`.
            assert_eq!(rendered["echoed"], "acme-large");
            assert_eq!(rendered["output"], "hi there");
        });
    }

    #[test]
    fn a_key_must_match_the_reference_pattern() {
        assert!(valid_key("demo"));
        assert!(valid_key("my-plugin_2"));
        assert!(!valid_key(""));
        assert!(!valid_key("Demo"));          // uppercase
        assert!(!valid_key("-leading"));      // must start alphanumeric
        assert!(!valid_key("has space"));
        assert!(!valid_key(&"a".repeat(31))); // reference caps at 30
        assert!(valid_key(&"a".repeat(30)));
    }

    #[test]
    fn a_manifest_says_which_hooks_it_implements() {
        let manifest = PluginManifest {
            api_version: API_VERSION_1,
            key: "demo".into(),
            name: "Demo".into(),
            version: "1.0.0".into(),
            description: String::new(),
            modes: vec![PluginMode {
                name: "openai".into(),
                hook: "convertRequest".into(),
                supports: vec!["openai".into()],
            }],
            protocols: Vec::new(),
        };
        assert!(manifest.implements("convertRequest"));
        assert!(!manifest.implements("convertResponse"));
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn an_unknown_api_version_is_refused() {
        let manifest = PluginManifest {
            api_version: 99,
            key: "demo".into(),
            name: String::new(),
            version: String::new(),
            description: String::new(),
            modes: Vec::new(),
            protocols: Vec::new(),
        };
        assert!(matches!(
            manifest.validate(),
            Err(PluginError::UnsupportedApiVersion { found: 99, .. })
        ));
    }

    #[test]
    fn a_plugin_loads_and_its_hook_round_trips_json() {
        let host = PluginHost::start();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let manifest = host
                .load(DEMO.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("load");
            assert_eq!(manifest.key, "demo");
            assert!(manifest.implements("convertRequest"));

            let out = host
                .call_hook(
                    "demo",
                    "convertRequest",
                    serde_json::json!({"model": "gpt-4o", "tokens": 7}),
                    DEFAULT_CALL_TIMEOUT,
                )
                .await
                .expect("hook");
            assert_eq!(out["model"], "GPT-4O");
            assert_eq!(out["seen"], 7);
        });
    }

    #[test]
    fn bad_plugins_are_refused_and_say_why() {
        let host = PluginHost::start();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            // No register call.
            let error = host
                .load("var x = 1;".to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect_err("must not load");
            assert!(matches!(error, PluginError::Load(_)), "{error:?}");

            // A manifest with an unsupported version is caught before the key is
            // trusted for anything.
            let bad_version = "register({apiVersion: 2, key: 'x', modes: []});";
            let error = host
                .load(bad_version.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect_err("must not load");
            assert!(
                matches!(error, PluginError::UnsupportedApiVersion { .. }),
                "{error:?}"
            );

            // A syntax error is a load failure, not a panic.
            let error = host
                .load("this is not javascript".to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect_err("must not load");
            assert!(matches!(error, PluginError::Load(_)), "{error:?}");

            // A hook on a plugin that never loaded is refused.
            let error = host
                .call_hook("nobody", "convertRequest", serde_json::json!({}), DEFAULT_CALL_TIMEOUT)
                .await
                .expect_err("must not run");
            assert!(matches!(error, PluginError::NoSuchHook { .. }), "{error:?}");
        });
    }

    #[test]
    fn a_hook_that_never_returns_is_cut_off() {
        let host = PluginHost::start();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let spinning = r#"
                register({apiVersion: 1, key: "spin", modes: [{name: "m", hook: "h"}]});
                var plugins = { spin: { h: function () { while (true) {} } } };
            "#;
            host.load(spinning.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("load");
            // The engine thread is now wedged; the caller must still be released.
            let started = std::time::Instant::now();
            let error = host
                .call_hook("spin", "h", serde_json::json!({}), Duration::from_millis(300))
                .await
                .expect_err("must time out");
            assert!(matches!(error, PluginError::Timeout(_)), "{error:?}");
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the caller waited far longer than the timeout"
            );
        });
    }
}
