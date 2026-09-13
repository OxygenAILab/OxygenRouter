//! Embed WebUI static assets at compile time (built by Vite into dist/)
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

#[cfg(feature = "embed-webui")]
pub static WEBUI_DIST: include_dir::Dir<'static> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/web/dist");

#[cfg(not(feature = "embed-webui"))]
pub const WEBUI_DIST_NOT_EMBEDDED: () = ();

#[cfg(feature = "embed-webui")]
pub fn serve_static(path: &str) -> Option<(Vec<u8>, &'static str)> {
    use include_dir::DirEntry;
    fn find_file<'a>(
        dir: &'a include_dir::Dir<'static>,
        name: &str,
    ) -> Option<&'a include_dir::File<'static>> {
        for entry in dir.entries() {
            match entry {
                DirEntry::File(f)
                    if f.path().file_name().and_then(|n| n.to_str()) == Some(name) =>
                {
                    return Some(f)
                }
                DirEntry::Dir(sub) => {
                    if let Some(found) = find_file(sub, name) {
                        return Some(found);
                    }
                }
                _ => {}
            }
        }
        None
    }
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }
    let file = find_file(&WEBUI_DIST, parts.last()?)?;
    let mime = match file.path().extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("mjs") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    };
    Some((file.contents().to_vec(), mime))
}

#[cfg(not(feature = "embed-webui"))]
pub fn serve_static(_path: &str) -> Option<(Vec<u8>, &'static str)> {
    None
}

/// Build an `axum::Response` for the given path (with `/ui` already stripped).
/// Falls back to `index.html` for unknown sub-paths (SPA routing).
pub fn serve_path(path: &str) -> axum::response::Response {
    serve_internal(path)
}

/// Build an `axum::Response` for the given request URI, with SPA fallback
/// to `index.html` for unknown sub-paths. Returns `404 Not Found` if
/// the embedded WebUI is not available (e.g. feature flag disabled).
pub fn serve_uri(uri: &axum::http::Uri) -> axum::response::Response {
    let raw = uri.path();
    let stripped = raw
        .trim_start_matches('/')
        .trim_end_matches('/')
        .to_string();
    serve_internal(&stripped)
}

fn serve_internal(stripped: &str) -> axum::response::Response {
    use axum::{
        http::StatusCode,
        response::{IntoResponse, Response},
    };

    if stripped.is_empty() {
        if let Some((body, mime)) = serve_static("index.html") {
            return Response::builder()
                .status(StatusCode::OK)
                .header("content-type", mime)
                .body(axum::body::Body::from(body))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    }

    if let Some((body, mime)) = serve_static(stripped) {
        return Response::builder()
            .status(StatusCode::OK)
            .header("content-type", mime)
            .body(axum::body::Body::from(body))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }

    // SPA fallback: any non-asset path that has no file → serve index.html.
    if let Some((body, mime)) = serve_static("index.html") {
        return Response::builder()
            .status(StatusCode::OK)
            .header("content-type", mime)
            .body(axum::body::Body::from(body))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }

    not_found_response()
}

fn not_found_response() -> axum::response::Response {
    use axum::{http::StatusCode, response::IntoResponse};
    let body = serde_json::json!({
        "error": {
            "message": "webui not bundled in this binary — rebuild with the `embed-webui` feature (or run `npm run dev` for the Vite dev server).",
            "type": "not_found_error",
        }
    });
    (
        StatusCode::NOT_FOUND,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&body).unwrap_or_default(),
    )
        .into_response()
}
