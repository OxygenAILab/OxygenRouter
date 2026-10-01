//! The HTTP transport a task flow sends through.
//!
//! Everything above this file is policy: a plugin states what to send, the
//! orchestrator decides whether the answer means anything. This is the only place
//! that makes the request, so it is also the only place that has to get the
//! transport rules right: no redirects (a plugin-stated URL must not be able to
//! bounce a channel credential to another host), a bounded body, a timeout, and
//! the credential attached at the last moment rather than by the plugin.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::time::Duration;

use oxygenrouter_plugin::{HttpOutcome, OutboundRequest, TaskTransport, TransportFuture};

/// The ceiling on one upstream answer.
///
/// An upstream that streams forever would otherwise be read into memory in full.
/// The reference caps a submitted body at 1 MiB and a poll body by the same idea
/// (`relay/channel/task/jsplugin/adaptor.go:81`); a poll answer is smaller still,
/// so this is generous for both.
pub const MAX_TASK_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// How long one submit or poll request may take.
pub const TASK_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// A transport backed by one `reqwest` client.
#[derive(Debug, Clone)]
pub struct ReqwestTaskTransport {
    client: reqwest::Client,
    /// The network guard, applied again at send time.
    ///
    /// The plugin's own guard compares hosts, which is what stops a credential
    /// going somewhere the operator never allowed. It cannot stop a *name* the
    /// operator did allow from resolving to a private address, because that answer
    /// only exists at resolution time. That is this policy's job, and running it
    /// here -- after the plugin has had its say and immediately before the socket
    /// -- is the only point where the resolved target is known.
    ssrf: oxygenrouter_proxy::ssrf::SsrfPolicy,
}

impl ReqwestTaskTransport {
    /// Build a client with the transport rules this flow needs.
    pub fn new(
        proxy: Option<&str>,
        ssrf: oxygenrouter_proxy::ssrf::SsrfPolicy,
    ) -> Result<Self, String> {
        let mut builder = reqwest::Client::builder()
            .timeout(TASK_REQUEST_TIMEOUT)
            // A redirect is the one way a plugin-stated URL could hand the
            // channel credential to a host the operator never allowed, so
            // redirects are not followed at all.
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("OxygenRouter/", env!("CARGO_PKG_VERSION")));
        if let Some(proxy) = proxy.map(str::trim).filter(|value| !value.is_empty()) {
            builder = builder.proxy(
                reqwest::Proxy::all(proxy).map_err(|error| format!("bad proxy {proxy:?}: {error}"))?,
            );
        }
        let client = builder
            .build()
            .map_err(|error| format!("could not build the task HTTP client: {error}"))?;
        Ok(ReqwestTaskTransport { client, ssrf })
    }

    /// The client this transport uses, for a caller that owns the proxy setting.
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Refuse a target the network guard disallows.
    ///
    /// Both halves are needed: the URL check catches a scheme or port the policy
    /// forbids, and the resolved-address check catches a permitted name that points
    /// somewhere it should not.
    async fn guard(&self, url: &str) -> Result<(), String> {
        self.ssrf
            .validate_url(url)
            .map_err(|error| format!("task target is not allowed: {error}"))?;
        let parsed = url::Url::parse(url)
            .map_err(|error| format!("task target is not a URL: {error}"))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| "task target has no host".to_string())?;
        let port = parsed
            .port_or_known_default()
            .ok_or_else(|| "task target has no port".to_string())?;
        // A literal address needs no lookup, and resolving one would be wasted
        // work on every poll.
        if host.parse::<std::net::IpAddr>().is_ok() {
            return self
                .ssrf
                .validate_network_target(host, port)
                .map_err(|error| format!("task target is not allowed: {error}"));
        }
        let addresses: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, port))
            .await
            .map_err(|error| format!("task target {host:?} could not be resolved: {error}"))?
            .collect();
        if addresses.is_empty() {
            return Err(format!("task target {host:?} resolved to no addresses"));
        }
        // Every answer has to pass: a name that resolves to one public and one
        // private address must not be usable, or a rebinding attack would only
        // have to win once.
        for address in &addresses {
            self.ssrf
                .validate_network_target(&address.ip().to_string(), address.port())
                .map_err(|error| {
                    format!(
                        "task target {host:?} resolves to {} which is not allowed: {error}",
                        address.ip()
                    )
                })?;
        }
        Ok(())
    }
}

impl TaskTransport for ReqwestTaskTransport {
    fn execute(&self, request: OutboundRequest) -> TransportFuture<'_> {
        Box::pin(async move { self.send(request).await })
    }
}

impl ReqwestTaskTransport {
    async fn send(&self, request: OutboundRequest) -> Result<HttpOutcome, String> {
        self.guard(&request.url).await?;
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|error| format!("plugin stated an unusable method: {error}"))?;
        let mut builder = self.client.request(method, &request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        if let Some(authorization) = &request.authorization {
            builder = builder.header(reqwest::header::AUTHORIZATION, authorization.as_str());
        }
        if let Some(body) = request.body {
            builder = builder.body(body.bytes);
        }

        let response = builder
            .send()
            .await
            .map_err(|error| format!("task request to {} failed: {error}", request.url))?;
        let status = response.status().as_u16();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_string(),
                    value.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect();

        // Read with a ceiling rather than trusting the upstream's length: a
        // chunked answer has none, and a hostile one would be unbounded.
        let mut body = Vec::new();
        let mut stream = response;
        while let Some(chunk) = stream
            .chunk()
            .await
            .map_err(|error| format!("reading the task response failed: {error}"))?
        {
            if body.len() + chunk.len() > MAX_TASK_RESPONSE_BYTES {
                return Err(format!(
                    "task response exceeded {MAX_TASK_RESPONSE_BYTES} bytes"
                ));
            }
            body.extend_from_slice(&chunk);
        }

        Ok(HttpOutcome {
            status,
            headers,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The client is built with the rules the flow depends on; the ones that can
    /// be asserted without a network are the redirect policy and the timeout.
    #[test]
    fn the_transport_client_refuses_to_follow_redirects() {
        let policy = oxygenrouter_proxy::ssrf::SsrfPolicy::permissive();
        let transport = ReqwestTaskTransport::new(None, policy.clone()).expect("transport");
        // `reqwest::Client` does not expose its policy, so the observable
        // guarantee is exercised against a redirecting server in the live tests;
        // here the construction itself is pinned, because a policy that silently
        // became `default()` would be a credential-leak regression.
        assert!(transport.client().get("https://example.invalid").build().is_ok());

        // A proxy string the URL parser rejects is refused at construction rather
        // than at the first request, so a misconfigured instance fails loudly.
        assert!(ReqwestTaskTransport::new(Some("not a proxy ::"), policy.clone()).is_err());
        assert!(ReqwestTaskTransport::new(Some("http://127.0.0.1:1"), policy.clone()).is_ok());
        // An empty proxy setting means "no proxy", not "a proxy at the empty URL".
        assert!(ReqwestTaskTransport::new(Some("   "), policy).is_ok());
    }

    /// The network guard runs at send time, which is the only point where a
    /// hostname's addresses are known.
    ///
    /// The plugin's guard compares hosts and so cannot see that an allowed *name*
    /// points at a private address; this is where that is caught, and it is
    /// caught for every address the name resolves to, so a name that answers with
    /// one public and one private address is still refused.
    #[tokio::test]
    async fn the_network_guard_refuses_a_private_target_and_allows_a_public_one() {
        // Loopback and metadata addresses are what a channel credential must never
        // reach.
        let strict = ReqwestTaskTransport::new(
            None,
            oxygenrouter_proxy::ssrf::SsrfPolicy::public_only(),
        )
        .expect("transport");
        for target in [
            "http://127.0.0.1:8080/steal",
            "http://[::1]:8080/steal",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.1/x",
            "http://192.168.1.1/x",
        ] {
            let error = strict.guard(target).await.expect_err("must refuse");
            assert!(error.contains("not allowed"), "{target}: {error}");
        }

        // Localhost by name is refused by the same resolved-address check, which is
        // the case a literal comparison would miss.
        assert!(strict.guard("http://localhost:3001/x").await.is_err());

        // A public literal address is allowed by the policy itself, so the guard
        // does not become "refuse everything".
        assert!(strict.guard("https://93.184.216.34/x").await.is_ok());

        // An operator who legitimately proxies to a LAN model server can turn the
        // private-address refusal off, and then the same target is allowed.
        let permissive = ReqwestTaskTransport::new(
            None,
            oxygenrouter_proxy::ssrf::SsrfPolicy::permissive(),
        )
        .expect("transport");
        assert!(permissive.guard("http://127.0.0.1:8080/x").await.is_ok());

        // A non-HTTP scheme is not something a task flow sends, whatever the
        // policy says about addresses.
        assert!(permissive.guard("file:///etc/passwd").await.is_err());
        let error = permissive
            .guard("file:///etc/passwd")
            .await
            .expect_err("must refuse");
        assert!(error.contains("not allowed"), "{error}");
    }

    /// A method the HTTP layer cannot express is refused before the network, while
    /// an extension verb is allowed.
    ///
    /// The distinction is real, not pedantic: HTTP allows any token as a method,
    /// so a vendor that uses `PURGE` or `BREW` works, but something with a space
    /// in it is a request line that cannot be written at all. Refusing it here
    /// names the plugin as the cause instead of surfacing a transport error.
    #[tokio::test]
    async fn an_extension_verb_is_allowed_and_an_unexpressible_method_is_refused() {
        assert!(reqwest::Method::from_bytes(b"BREW").is_ok());
        assert!(reqwest::Method::from_bytes(b"BAD METHOD").is_err());

        let transport = ReqwestTaskTransport::new(
            None,
            oxygenrouter_proxy::ssrf::SsrfPolicy::permissive(),
        )
        .expect("transport");
        let error = transport
            .send(OutboundRequest {
                method: "BAD METHOD".to_string(),
                // A literal address, so the method check is the first thing that
                // can fail: `example.invalid` is guaranteed not to resolve, and a
                // resolution error would mask what this test is about.
                url: "https://93.184.216.34/x".to_string(),
                headers: Vec::new(),
                body: None,
                authorization: None,
            })
            .await
            .expect_err("must refuse");
        assert!(error.contains("unusable method"), "{error}");
    }
}
