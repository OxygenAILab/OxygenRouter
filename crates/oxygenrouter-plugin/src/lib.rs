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
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Call::Hook {
                key: key.to_string(),
                hook: hook.to_string(),
                input: serde_json::to_string(&input).unwrap_or_else(|_| "null".to_string()),
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
                let _ = reply.send(engine.call(&key, &hook, &input));
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
        self.loaded.insert(manifest.key.clone());
        Ok(manifest)
    }

    /// Call `hook` on the plugin named `key`, passing and receiving JSON.
    fn call(&mut self, key: &str, hook: &str, input: &str) -> Result<String, PluginError> {
        use boa_engine::Source;

        if !self.loaded.contains(key) {
            return Err(PluginError::NoSuchHook {
                key: key.to_string(),
                hook: hook.to_string(),
            });
        }
        // The hook is invoked as a method on the plugin's exported object, which
        // is how the reference lays its plugins out; a plugin that does not
        // export the hook is reported rather than treated as a no-op, because a
        // silently missing hook is a routing surprise.
        let script = format!(
            "(function () {{\n\
             var __input = JSON.parse({input});\n\
             var __plugin = (typeof plugins !== 'undefined' && plugins[{key_json}]) ||\n\
                            (typeof module !== 'undefined' && module.exports && module.exports[{key_json}]) ||\n\
                            (typeof exports !== 'undefined' && exports[{key_json}]);\n\
             if (!__plugin) throw new Error('plugin ' + {key_json} + ' did not export an object');\n\
             var __hook = __plugin[{hook_json}];\n\
             if (typeof __hook !== 'function') throw new Error('plugin ' + {key_json} + ' has no hook ' + {hook_json});\n\
             return JSON.stringify(__hook(__input));\n\
             }})()",
            input = serde_json::to_string(input).unwrap_or_else(|_| "\"null\"".to_string()),
            key_json = serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string()),
            hook_json = serde_json::to_string(hook).unwrap_or_else(|_| "\"\"".to_string()),
        );

        let value = self
            .context
            .eval(Source::from_bytes(script.as_bytes()))
            .map_err(|error| PluginError::Hook {
                key: key.to_string(),
                hook: hook.to_string(),
                message: describe(&error),
            })?;
        let s = value
            .to_string(&mut self.context)
            .map_err(|error| PluginError::Hook {
                key: key.to_string(),
                hook: hook.to_string(),
                message: describe(&error),
            })?
            .to_std_string_escaped();
        Ok(s)
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
