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

/// The launcher prelude for the script contract.
///
/// Defines `register` so a plugin written as a plain script can state its
/// manifest. It lives in the script rather than as a host callback because a
/// callback would have to capture shared state across Boa's garbage collector,
/// and it resets the slot so a plugin that forgets to register cannot inherit
/// the previous plugin's manifest.
const PRELUDE: &str = "\
    var __ORM_MANIFEST = null;\n\
    function register(manifest) { __ORM_MANIFEST = manifest; }\n";

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

/// One loaded plugin: the module, and the manifest it exported.
///
/// The module is kept rather than its namespace because a namespace is tied to
/// the context that evaluated it and the host must be able to call members of a
/// plugin loaded long ago.
struct Loaded {
    /// `Some` for the module form, `None` for the script form.
    ///
    /// The distinction is not cosmetic: a module's top-level `var` is
    /// module-scoped and never becomes a global property, so a script-form
    /// plugin's `protocols` can only be found if the script was evaluated *as a
    /// script*. Both forms are therefore kept apart rather than normalised,
    /// because normalising them would silently break one of them.
    module: Option<boa_engine::Module>,
    manifest: PluginManifest,
}

/// The engine thread's private state. Never leaves this thread.
struct Engine {
    context: boa_engine::Context,
    /// Plugins that loaded successfully, by plugin key and by each protocol name
    /// they claim. The second spelling is how the reference addresses protocol
    /// members (`protocols.openai_responses.*`), so both are addressable and a
    /// call site does not have to know which one it holds.
    loaded: std::collections::HashMap<String, Loaded>,
    /// Module path counter, so each loaded plugin has its own identity inside the
    /// loader and cannot shadow another's exports.
    next_module: u32,
}

impl Engine {
    fn new() -> Self {
        Self {
            context: boa_engine::Context::default(),
            loaded: std::collections::HashMap::new(),
            next_module: 0,
        }
    }

    /// Load a plugin and return the manifest it declared.
    ///
    /// The reference compiles plugins as ES modules (`sobek.ParseModule`,
    /// `pkg/jsplugin/engine.go:145`) and reads `meta` from the module's exports,
    /// so a plugin written for it is a module exporting `meta` and `protocols`.
    /// The script form — `register(meta)` plus a `protocols` assignment — is
    /// accepted too, and is evaluated *as a script* rather than as a module,
    /// because a module's top-level `var` is module-scoped and never becomes a
    /// global property; normalising the two forms would silently break one.
    fn load(&mut self, source: &str) -> Result<PluginManifest, PluginError> {
        // Two refusals the reference makes and this host keeps, both because the
        // plugin is untrusted. `import` would pull in code the operator never
        // reviewed; a `sourceMappingURL` comment would let the parser read
        // arbitrary server files while reporting an error.
        if let Some(specifier) = forbidden_import(source) {
            return Err(PluginError::Load(format!(
                "plugin imports are disabled: {specifier}"
            )));
        }
        if has_source_map_directive(source) {
            return Err(PluginError::Load(
                "plugin source maps are disabled: a sourceMappingURL directive can make the parser read server files"
                    .to_string(),
            ));
        }
        // The script contract's `register` must exist before the body runs, since
        // a script-form plugin calls it during evaluation.
        self.context
            .eval(boa_engine::Source::from_bytes(PRELUDE))
            .map_err(|error| PluginError::Load(describe(&error)))?;

        self.next_module += 1;
        let path = format!("plugin{}.js", self.next_module);
        let is_module = looks_like_module(source);

        let (module, manifest, exported) = if is_module {
            let module = boa_engine::Module::parse(
                boa_engine::Source::from_bytes(source.as_bytes())
                    .with_path(std::path::Path::new(&path)),
                None,
                &mut self.context,
            )
            .map_err(|error| PluginError::Load(describe(&error)))?;
            // Linking is where the engine checks that every import resolves. The
            // default loader resolves none, so a plugin reaching for a module
            // fails here with a message naming it.
            module
                .link(&mut self.context)
                .map_err(|error| PluginError::Load(describe(&error)))?;
            let promise = module
                .evaluate(&mut self.context)
                .map_err(|error| PluginError::Load(describe(&error)))?;
            // A module body may await, so evaluation yields a promise. No clock is
            // installed, so only the body's own synchronous microtasks can run --
            // but a throw inside the body surfaces here as a *rejected promise*
            // rather than an evaluate() error, and ignoring it would make a
            // broken plugin look like one that declared nothing.
            let _ = self
                .context
                .run_jobs()
                .map_err(|error| PluginError::Load(describe(&error)))?;
            let as_value: boa_engine::JsValue = promise.into();
            let Some(handle) = as_value.as_promise() else {
                return Err(PluginError::Load(
                    "the engine did not return a promise for a module body".to_string(),
                ));
            };
            match handle.state() {
                boa_engine::builtins::promise::PromiseState::Rejected(reason) => {
                    let text = reason
                        .to_string(&mut self.context)
                        .map(|s| s.to_std_string_escaped())
                        .unwrap_or_else(|_| "plugin threw while loading".to_string());
                    return Err(PluginError::Load(describe_text(&text)));
                }
                boa_engine::builtins::promise::PromiseState::Pending => {
                    return Err(PluginError::Load(
                        "plugin top-level await never settled; plugins must be synchronous"
                            .to_string(),
                    ));
                }
                boa_engine::builtins::promise::PromiseState::Fulfilled(_) => {}
            }
            let manifest = self.manifest_from_module(&module)?;
            let exported = self.exported_protocol_members(Some(&module));
            (Some(module), manifest, exported)
        } else {
            self.context
                .eval(boa_engine::Source::from_bytes(source.as_bytes()))
                .map_err(|error| PluginError::Load(describe(&error)))?;
            let manifest = self.manifest_from_register()?;
            let exported = self.exported_protocol_members(None);
            (None, manifest, exported)
        };

        manifest.validate()?;
        let problems = validate_protocol_claims(&manifest.protocols, &manifest.key, &exported);
        if !problems.is_empty() {
            return Err(PluginError::Load(problems.join("; ")));
        }

        // Addressable by plugin key and by every protocol the plugin claims,
        // because those are the two names a caller may hold.
        let mut names = vec![manifest.key.clone()];
        names.extend(manifest.protocols.iter().map(|c| c.name.clone()));
        for name in names {
            self.loaded.insert(
                name,
                Loaded {
                    module: module.clone(),
                    manifest: manifest.clone(),
                },
            );
        }
        Ok(manifest)
    }

    /// The manifest a module declared through its `meta` export.
    fn manifest_from_module(
        &mut self,
        module: &boa_engine::Module,
    ) -> Result<PluginManifest, PluginError> {
        let namespace = module.namespace(&mut self.context);
        let value = namespace.get(boa_engine::js_string!("meta"), &mut self.context);
        match value {
            Ok(value) if !value.is_undefined() => {
                let json = value
                    .to_json(&mut self.context)
                    .map_err(|error| PluginError::BadManifest(describe(&error)))?;
                serde_json::from_value(
                    serde_json::to_value(json).unwrap_or(serde_json::Value::Null),
                )
                .map_err(|error| PluginError::BadManifest(error.to_string()))
            }
            _ => Err(PluginError::Load(
                "module must export `meta`".to_string(),
            )),
        }
    }

    /// The manifest a script declared by calling `register`.
    fn manifest_from_register(&mut self) -> Result<PluginManifest, PluginError> {
        use boa_engine::Source;

        let raw = self
            .context
            .eval(Source::from_bytes(
                "(typeof __ORM_MANIFEST === 'undefined' || __ORM_MANIFEST === null) ? 'null' : JSON.stringify(__ORM_MANIFEST)",
            ))
            .map_err(|error| PluginError::Load(describe(&error)))?
            .to_string(&mut self.context)
            .map_err(|error| PluginError::Load(describe(&error)))?
            .to_std_string_escaped();
        if raw == "null" || raw == "undefined" {
            return Err(PluginError::Load(
                "plugin neither exported `meta` nor called register(manifest)".to_string(),
            ));
        }
        serde_json::from_str(&raw).map_err(|error| PluginError::BadManifest(error.to_string()))
    }

    /// The plugin's `protocols` object, from a module export or the global a
    /// script assigns.
    fn protocols_object(
        &mut self,
        module: Option<&boa_engine::Module>,
    ) -> Option<boa_engine::JsObject> {
        if let Some(module) = module {
            let namespace = module.namespace(&mut self.context);
            if let Ok(value) = namespace.get(boa_engine::js_string!("protocols"), &mut self.context)
            {
                if let Ok(obj) = value.to_object(&mut self.context) {
                    return Some(obj);
                }
            }
        }
        let global = self.context.global_object();
        let value = global
            .get(boa_engine::js_string!("protocols"), &mut self.context)
            .ok()?;
        value.to_object(&mut self.context).ok()
    }

    /// Every protocol member the plugin provides as a function.
    fn exported_protocol_members(
        &mut self,
        module: Option<&boa_engine::Module>,
    ) -> Vec<String> {
        let Some(obj) = self.protocols_object(module) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for protocol in HOST_PROTOCOLS {
            let Ok(entry) = obj.get(boa_engine::js_string!(protocol.name), &mut self.context) else {
                continue;
            };
            let Ok(entry) = entry.to_object(&mut self.context) else {
                continue;
            };
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
                let Ok(value) = entry.get(boa_engine::js_string!(member), &mut self.context) else {
                    continue;
                };
                if value.as_callable().is_some() {
                    out.push(member.to_string());
                }
            }
        }
        out
    }

    /// Call `protocols.<name>.<member>` with JSON arguments, returning JSON.
    ///
    /// The member is reached through the module's own `protocols` export, so the
    /// call site matches the shape a plugin is written in and a plugin authored
    /// for the reference works unchanged.
    fn call_member(
        &mut self,
        key: &str,
        member: &str,
        args: &[serde_json::Value],
    ) -> Result<String, PluginError> {
        // Cloned out of the map before anything else borrows `self` mutably: the
        // manifest is needed for the fallback lookup and the module for the call,
        // and holding the map entry across either would trap the borrow.
        let Some((module, plugin_key)) = self
            .loaded
            .get(key)
            .map(|loaded| (loaded.module.clone(), loaded.manifest.key.clone()))
        else {
            return Err(PluginError::NoSuchHook {
                key: key.to_string(),
                hook: member.to_string(),
            });
        };

        let Some(protocol) = self.protocols_object(module.as_ref()) else {
            return Err(PluginError::Hook {
                key: key.to_string(),
                hook: member.to_string(),
                message: "plugin exports no protocols object".to_string(),
            });
        };
        // The caller may hold either the plugin's key or the protocol's name, so
        // both are tried before giving up.
        let entry = {
            let by_key = protocol.get(boa_engine::js_string!(key), &mut self.context);
            match by_key {
                Ok(value) if !value.is_undefined() => Ok(value),
                _ => {
                    // Fall back to the plugin's own key when the caller passed a
                    // protocol name that this plugin does not own.
                    protocol.get(boa_engine::js_string!(plugin_key.as_str()), &mut self.context)
                }
            }
        };
        let entry = entry
            .map_err(|error| PluginError::Hook {
                key: key.to_string(),
                hook: member.to_string(),
                message: describe(&error),
            })?
            .to_object(&mut self.context)
            .map_err(|error| PluginError::Hook {
                key: key.to_string(),
                hook: member.to_string(),
                message: format!(
                    "plugin {} exports no object for {:?}: {}",
                    plugin_key,
                    key,
                    describe(&error)
                ),
            })?;

        let function = entry
            .get(boa_engine::js_string!(member), &mut self.context)
            .map_err(|error| PluginError::Hook {
                key: key.to_string(),
                hook: member.to_string(),
                message: describe(&error),
            })?;
        let Some(callable) = function.as_callable() else {
            return Err(PluginError::Hook {
                key: key.to_string(),
                hook: member.to_string(),
                message: format!("plugin has no member {member:?}"),
            });
        };

        let mut argv = Vec::new();
        for arg in args {
            let value = boa_engine::JsValue::from_json(
                &serde_json::to_value(arg).unwrap_or(serde_json::Value::Null),
                &mut self.context,
            )
            .map_err(|error| PluginError::Hook {
                key: key.to_string(),
                hook: member.to_string(),
                message: describe(&error),
            })?;
            argv.push(value);
        }
        let result = callable
            .call(&boa_engine::JsValue::undefined(), &argv, &mut self.context)
            .map_err(|error| PluginError::Hook {
                key: key.to_string(),
                hook: member.to_string(),
                message: describe(&error),
            })?;
        let json = result
            .to_json(&mut self.context)
            .map_err(|error| PluginError::Hook {
                key: key.to_string(),
                hook: member.to_string(),
                message: describe(&error),
            })?;
        Ok(serde_json::to_string(&serde_json::to_value(json).unwrap_or(serde_json::Value::Null))
            .unwrap_or_else(|_| "null".to_string()))
    }
}

/// The first `import`/`export ... from` specifier in `source`, if any.
///
/// The keyword is located in a copy with comments and string bodies blanked, so
/// a plugin describing an import in prose is not refused for it, but the
/// specifier itself is read from the **original** text: the blanked copy no
/// longer contains it, which is a mistake this function made in its first form.
/// The reference draws the same distinction before its own scan
/// (`pkg/jsplugin/engine.go:136`).
fn forbidden_import(source: &str) -> Option<String> {
    let stripped = strip_comments_and_strings(source);

    let mut from = 0usize;
    while let Some(rel) = stripped[from..].find("import") {
        let at = from + rel;
        if keyword_boundary_ok(&stripped, at, "import") {
            if let Some(spec) = read_import_specifier(&source[at..]) {
                return Some(spec);
            }
        }
        from = at + "import".len();
    }

    // `export ... from "..."` re-exports another module's code, which is the same
    // hazard by another spelling.
    let mut from = 0usize;
    while let Some(rel) = stripped[from..].find("export") {
        let at = from + rel;
        if keyword_boundary_ok(&stripped, at, "export") {
            if let Some(spec) = read_export_from_specifier(&source[at..]) {
                return Some(spec);
            }
        }
        from = at + "export".len();
    }
    None
}

/// True when `keyword` at `at` is a standalone word rather than part of a longer
/// identifier or a member access.
fn keyword_boundary_ok(text: &str, at: usize, keyword: &str) -> bool {
    let before_ok = text[..at]
        .chars()
        .next_back()
        .map(|c| !c.is_alphanumeric() && c != '_' && c != '.')
        .unwrap_or(true);
    let after_ok = text[at + keyword.len()..]
        .chars()
        .next()
        .map(|c| !c.is_alphanumeric() && c != '_')
        .unwrap_or(true);
    before_ok && after_ok
}

/// The specifier of an `import` statement beginning at `rest`.
fn read_import_specifier(rest: &str) -> Option<String> {
    let after = rest.get("import".len()..)?.trim_start();
    // A dynamic import loads code just as surely as a static one, and is refused
    // without trying to name what it loads.
    if after.starts_with('(') {
        return Some("(dynamic import)".to_string());
    }
    // `import "x"` has the specifier immediately after the keyword.
    if let Some(spec) = read_quoted(after) {
        return Some(spec);
    }
    let from = after.find(" from ")?;
    read_quoted(&after[from + " from ".len()..])
}

fn read_export_from_specifier(rest: &str) -> Option<String> {
    let after = rest.get("export".len()..)?;
    let from = after.find(" from ")?;
    read_quoted(&after[from + " from ".len()..])
}

/// The contents of the quoted string `s` begins with, if it begins with one.
fn read_quoted(s: &str) -> Option<String> {
    let s = s.trim_start();
    let quote = s.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let body = s.get(1..)?;
    let end = body.find(quote)?;
    Some(body[..end].to_string())
}

/// Whether a plugin source should be parsed as a module.
///
/// Module syntax is what decides it: `export` or a static `import` is only legal
/// in a module, so a source carrying either must be one, and a source carrying
/// neither is treated as a script -- which is also the form the `register`
/// contract is written in.
fn looks_like_module(source: &str) -> bool {
    let stripped = strip_comments_and_strings(source);
    let has = |keyword: &str| {
        let mut from = 0usize;
        while let Some(rel) = stripped[from..].find(keyword) {
            let at = from + rel;
            if keyword_boundary_ok(&stripped, at, keyword) {
                return true;
            }
            from = at + keyword.len();
        }
        false
    };
    has("export") || has("import")
}

/// True when the source carries a `sourceMappingURL` directive.
fn has_source_map_directive(source: &str) -> bool {
    source.contains("sourceMappingURL")
}

/// Replace comment and string bodies with spaces, so a scan for keywords cannot
/// be fooled by prose.
///
/// Characters are replaced rather than removed to keep byte offsets meaningful,
/// and because a scanner that deletes text can splice two tokens into a third
/// that was never written.
fn strip_comments_and_strings(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    out[i] = b' ';
                    i += 1;
                }
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                out[i] = b' ';
                out[i + 1] = b' ';
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                        out[i] = b' ';
                        out[i + 1] = b' ';
                        i += 2;
                        break;
                    }
                    if bytes[i] != b'\n' {
                        out[i] = b' ';
                    }
                    i += 1;
                }
            }
            quote @ (b'"' | b'\'' | b'`') => {
                out[i] = b' ';
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        out[i] = b' ';
                        if i + 1 < bytes.len() {
                            out[i + 1] = b' ';
                        }
                        i += 2;
                        continue;
                    }
                    if bytes[i] == quote {
                        out[i] = b' ';
                        i += 1;
                        break;
                    }
                    if bytes[i] != b'\n' {
                        out[i] = b' ';
                    }
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Flatten and truncate a message the engine already rendered to text.
fn describe_text(text: &str) -> String {
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

    /// A plugin in the shape the reference actually writes them: an ES module
    /// exporting `meta` and `protocols`
    /// (`pkg/jsplugin/protocol_supports_test.go:13`).
    const DEMO: &str = r#"
        export const meta = {
            apiVersion: 1,
            key: "demo",
            name: "Demo Plugin",
            version: "1.0.0",
            description: "renders a fixed reply",
            protocols: [{ name: "openai_responses", supports: ["sync"] }]
        };
        export const protocols = {
            openai_responses: {
                decodeRequest: function (ctx) {
                    return { model: ctx.body.model, input: ctx.body.input };
                },
                renderFinal: function (ctx, task) {
                    return { id: task.id, model: ctx.body.model, output: task.output };
                }
            }
        };
    "#;

    /// A plain script is accepted too: `register(meta)` plus a `protocols` value.
    /// It costs nothing to support and is a reasonable thing to write.
    const SCRIPT_FORM: &str = r#"
        register({apiVersion:1, key:"scripted", name:"Scripted", version:"1.0.0",
            protocols:[{name:"openai_responses", supports:["sync"]}]});
        var protocols = { openai_responses: {
            decodeRequest: function (ctx) { return ctx; },
            renderFinal: function (ctx, task) { return task; }
        } };
    "#;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
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

    /// `import` pulls in code the operator never reviewed, in every spelling, so
    /// all of them are refused. Prose and strings about imports are not.
    #[test]
    fn imports_are_refused_but_prose_about_them_is_not() {
        assert_eq!(
            forbidden_import("import x from 'evil.js';").as_deref(),
            Some("evil.js")
        );
        assert_eq!(
            forbidden_import("import 'evil.js';").as_deref(),
            Some("evil.js")
        );
        assert_eq!(
            forbidden_import("import { a } from \"evil.js\";").as_deref(),
            Some("evil.js")
        );
        // Re-exporting another module's code is the same hazard.
        assert_eq!(
            forbidden_import("export { a } from 'evil.js';").as_deref(),
            Some("evil.js")
        );
        // Dynamic import is code loading by another name.
        assert!(forbidden_import("const m = import('evil.js');").is_some());

        // Comments and strings are stripped first, so these are clean.
        assert!(forbidden_import("// we import nothing here").is_none());
        assert!(forbidden_import("/* import x from 'evil.js' */").is_none());
        assert!(forbidden_import("const s = \"import x from 'evil.js'\";").is_none());
        // The word inside a longer identifier is not the keyword.
        assert!(forbidden_import("const a = obj.important;").is_none());
        assert!(forbidden_import("function reimport() { return 1; }").is_none());
    }

    /// A source-mapping directive can make the parser read arbitrary server
    /// files, so it is refused outright.
    #[test]
    fn source_mapping_directives_are_refused() {
        assert!(has_source_map_directive("//# sourceMappingURL=file:///etc/passwd"));
        assert!(!has_source_map_directive("export const meta = {};"));
    }

    /// The module contract end to end: an ES module loads, its `meta` becomes the
    /// manifest, and both protocol members answer with their own arity.
    #[test]
    fn a_module_plugin_loads_and_its_members_answer() {
        let host = PluginHost::start();
        runtime().block_on(async {
            let manifest = host
                .load(DEMO.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("load");
            assert_eq!(manifest.key, "demo");
            assert_eq!(manifest.name, "Demo Plugin");
            assert_eq!(manifest.protocols.len(), 1);
            assert_eq!(manifest.protocols[0].name, "openai_responses");
            assert_eq!(manifest.protocols[0].supports, vec!["sync".to_string()]);

            let ctx = serde_json::json!({
                "path": "/v1/responses",
                "method": "POST",
                "body": { "model": "acme-large", "input": "hello" }
            });
            // Addressed by protocol name, which is how the export is keyed.
            let upstream = host
                .decode_request("openai_responses", ctx.clone(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("decodeRequest");
            assert_eq!(upstream["model"], "acme-large");

            let task = serde_json::json!({ "id": "resp_1", "output": "hi there" });
            let rendered = host
                .render_final("openai_responses", ctx.clone(), task, DEFAULT_CALL_TIMEOUT)
                .await
                .expect("renderFinal");
            // The second argument arriving is what proves the arity; a
            // one-argument call would throw on `task.id`.
            assert_eq!(rendered["id"], "resp_1");
            assert_eq!(rendered["model"], "acme-large");
            assert_eq!(rendered["output"], "hi there");
        });
    }

    /// The script form is still accepted, because it is a reasonable thing to
    /// write and costs nothing to support.
    #[test]
    fn the_script_form_still_loads() {
        let host = PluginHost::start();
        runtime().block_on(async {
            let manifest = host
                .load(SCRIPT_FORM.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("load");
            assert_eq!(manifest.key, "scripted");
            let out = host
                .decode_request("openai_responses", serde_json::json!({"a": 1}), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("decodeRequest");
            assert_eq!(out["a"], 1);
        });
    }

    /// A module that throws while loading must report why. This is the case that
    /// a discarded evaluate() promise hides: the throw arrives as a rejected
    /// promise, not as an error from evaluate(), and the failure then looks like
    /// a plugin that simply declared nothing.
    #[test]
    fn a_module_that_throws_reports_the_reason() {
        let host = PluginHost::start();
        runtime().block_on(async {
            let blowing_up = "export const meta = undefinedFunction();";
            let error = host
                .load(blowing_up.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect_err("must not load");
            let text = error.to_string();
            assert!(!text.contains("declared no manifest"), "{text}");
            assert!(text.contains("undefinedFunction"), "{text}");
        });
    }

    /// An unknown protocol, or one whose members were not exported, is refused
    /// with a message naming the missing member. The host asks the engine what
    /// the source exports rather than trusting the manifest.
    #[test]
    fn protocol_claims_are_checked_against_what_the_source_exports() {
        let host = PluginHost::start();
        runtime().block_on(async {
            const MISSING: &str = r#"
                export const meta = { apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                    protocols:[{name:"openai_responses", supports:["stream"]}] };
                export const protocols = { openai_responses: {
                    decodeRequest: function (ctx) { return ctx; }
                } };
            "#;
            const UNKNOWN: &str = r#"
                export const meta = { apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                    protocols:[{name:"made_up", supports:["stream"]}] };
                export const protocols = { made_up: { decodeRequest: function(){return {};} } };
            "#;
            const NO_SUPPORTS: &str = r#"
                export const meta = { apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                    protocols:[{name:"openai_responses", supports:[]}] };
                export const protocols = { openai_responses: { decodeRequest: function(){return {};} } };
            "#;
            for (label, source, want) in [
                ("missing member", MISSING, "renderEvents"),
                ("unknown protocol", UNKNOWN, "does not serve"),
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

    #[test]
    fn an_unsupported_api_version_is_refused() {
        let host = PluginHost::start();
        runtime().block_on(async {
            let old = r#"
                export const meta = { apiVersion: 7, key:"old", name:"Old", version:"1.0.0" };
                export const protocols = {};
            "#;
            let error = host
                .load(old.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect_err("must not load");
            assert!(
                matches!(error, PluginError::UnsupportedApiVersion { found: 7, .. }),
                "{error:?}"
            );
        });
    }

    /// A plugin that throws at *call* time is reported, and a member that does
    /// not exist is distinguished from one that failed.
    #[test]
    fn call_time_failures_are_reported() {
        let host = PluginHost::start();
        runtime().block_on(async {
            host.load(DEMO.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("load");
            let error = host
                .decode_request("openai_responses", serde_json::json!({}), DEFAULT_CALL_TIMEOUT)
                .await
                .expect_err("no body");
            assert!(matches!(error, PluginError::Hook { .. }), "{error:?}");

            let error = host
                .call_hook("demo", "renderEvents", serde_json::json!({}), DEFAULT_CALL_TIMEOUT)
                .await
                .expect_err("not implemented");
            let text = error.to_string();
            assert!(text.contains("renderEvents"), "{text}");

            let error = host
                .call_hook("nobody", "decodeRequest", serde_json::json!({}), DEFAULT_CALL_TIMEOUT)
                .await
                .expect_err("unknown plugin");
            assert!(matches!(error, PluginError::NoSuchHook { .. }), "{error:?}");
        });
    }

    /// A hook that never returns is cut off and the caller is released. This is
    /// the only defence a gateway has against a plugin it did not write.
    #[test]
    fn a_hook_that_never_returns_is_cut_off() {
        let host = PluginHost::start();
        runtime().block_on(async {
            const SPINNING: &str = r#"
                export const meta = { apiVersion:1, key:"spin", name:"Spin", version:"1.0.0",
                    protocols:[{name:"openai_responses", supports:["sync"]}] };
                export const protocols = { openai_responses: {
                    decodeRequest: function () { while (true) {} },
                    renderFinal: function () { while (true) {} }
                } };
            "#;
            host.load(SPINNING.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("load");
            let started = std::time::Instant::now();
            let error = host
                .decode_request(
                    "openai_responses",
                    serde_json::json!({}),
                    Duration::from_millis(300),
                )
                .await
                .expect_err("must time out");
            assert!(matches!(error, PluginError::Timeout(_)), "{error:?}");
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the caller waited far longer than the timeout"
            );
        });
    }

    /// The host table is the contract, so its shape is worth pinning.
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
        // reason this protocol needs a plugin at all.
        assert_eq!(
            create.modes.iter().find(|m| m.name == "stream").unwrap().hook,
            "renderEvents"
        );
        assert_eq!(
            create.modes.iter().find(|m| m.name == "sync").unwrap().hook,
            "renderFinal"
        );

        // Validation reports one problem per (claimed form, missing member), so
        // an author who drops one claim still sees the other.
        let problems = validate_protocol_claims(
            &[ProtocolClaim {
                name: "openai_responses".into(),
                models: Vec::new(),
                supports: vec!["stream".into(), "sync".into()],
            }],
            "acme",
            &[],
        );
        assert_eq!(problems.len(), 4, "{problems:?}");
        for member in ["decodeRequest", "renderEvents", "renderFinal"] {
            assert!(
                problems.iter().any(|p| p.contains(member)),
                "{member} missing from {problems:?}"
            );
        }
        assert!(validate_protocol_claims(
            &[ProtocolClaim {
                name: "openai_responses".into(),
                models: Vec::new(),
                supports: vec!["stream".into(), "sync".into()],
            }],
            "acme",
            &[
                "decodeRequest".into(),
                "renderEvents".into(),
                "renderFinal".into()
            ],
        )
        .is_empty());
    }
}
