//! Static assets for the bridge: the API reference, the JS client, and the
//! example console.
//!
//! These are embedded in the binary rather than read from disk. The bridge has
//! to work from a signed `.app` bundle, from a dev checkout, and from `cargo
//! test`, and embedding removes every path-resolution failure mode at the cost
//! of ~1.7 MB in a binary that already links git2 and Tauri.
//!
//! Swagger UI is vendored (Apache-2.0) with its licence, notice and an
//! integrity manifest in `assets/bridge/docs/VENDOR.json`, so the reference
//! renders with no CDN and no network access at all.

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

const DOCS_INDEX: &str = include_str!("../../assets/bridge/docs/index.html");
const DOCS_INIT: &str = include_str!("../../assets/bridge/docs/init.js");
const DOCS_CSS: &str = include_str!("../../assets/bridge/docs/docs.css");
const SWAGGER_JS: &str = include_str!("../../assets/bridge/docs/swagger-ui-bundle.js");
const SWAGGER_CSS: &str = include_str!("../../assets/bridge/docs/swagger-ui.css");
const SWAGGER_LICENSE: &str = include_str!("../../assets/bridge/docs/LICENSE");
const SWAGGER_NOTICE: &str = include_str!("../../assets/bridge/docs/NOTICE");
const SWAGGER_JS_LICENSE: &str =
    include_str!("../../assets/bridge/docs/swagger-ui-bundle.js.LICENSE.txt");
const SWAGGER_VENDOR: &str = include_str!("../../assets/bridge/docs/VENDOR.json");

const CLIENT_MJS: &str = include_str!("../../assets/bridge/client.mjs");
const EXAMPLE_INDEX: &str = include_str!("../../assets/bridge/example/index.html");
const EXAMPLE_JS: &str = include_str!("../../assets/bridge/example/app.js");
const EXAMPLE_CSS: &str = include_str!("../../assets/bridge/example/style.css");

const HTML: &str = "text/html; charset=utf-8";
const JS: &str = "text/javascript; charset=utf-8";
const CSS: &str = "text/css; charset=utf-8";
const TEXT: &str = "text/plain; charset=utf-8";
const JSON: &str = "application/json; charset=utf-8";

fn serve(content_type: &'static str, body: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            // Assets change with the app, and the app is a local process; a
            // short cache keeps the docs page snappy without pinning a stale
            // bundle across an update.
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        body,
    )
        .into_response()
}

fn not_found(path: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, TEXT)],
        format!("No such bridge asset: {path}\n"),
    )
        .into_response()
}

/// Look one asset up by its path within `/docs`.
fn docs_file(path: &str) -> Option<(&'static str, &'static str)> {
    Some(match path {
        "index.html" | "" => (HTML, DOCS_INDEX),
        "init.js" => (JS, DOCS_INIT),
        "docs.css" => (CSS, DOCS_CSS),
        "swagger-ui-bundle.js" => (JS, SWAGGER_JS),
        "swagger-ui.css" => (CSS, SWAGGER_CSS),
        "swagger-ui-bundle.js.LICENSE.txt" => (TEXT, SWAGGER_JS_LICENSE),
        "LICENSE" => (TEXT, SWAGGER_LICENSE),
        "NOTICE" => (TEXT, SWAGGER_NOTICE),
        "VENDOR.json" => (JSON, SWAGGER_VENDOR),
        _ => return None,
    })
}

fn example_file(path: &str) -> Option<(&'static str, &'static str)> {
    Some(match path {
        "index.html" | "" => (HTML, EXAMPLE_INDEX),
        "app.js" => (JS, EXAMPLE_JS),
        "style.css" => (CSS, EXAMPLE_CSS),
        _ => return None,
    })
}

/// Redirect a directory-ish path to its trailing-slash form.
///
/// Both pages reference their assets relatively (`swagger-ui.css`, `app.js`),
/// so serving them at `/docs` makes the browser resolve every one against `/`
/// — the reference renders unstyled and scriptless, and the console does not
/// load at all. The redirect is the whole fix, and it has to use the request's
/// own path so it works identically under the `/v1` mount.
pub async fn redirect_to_slash(uri: axum::http::Uri) -> Response {
    let target = format!("{}/", uri.path());
    axum::response::Redirect::permanent(&target).into_response()
}

/// `GET /docs/` — the API reference.
pub async fn docs_index() -> Response {
    serve(HTML, DOCS_INDEX)
}

/// `GET /docs/{*path}`.
pub async fn docs_asset(axum::extract::Path(path): axum::extract::Path<String>) -> Response {
    match docs_file(path.trim_start_matches('/')) {
        Some((content_type, body)) => serve(content_type, body),
        None => not_found(&path),
    }
}

/// `GET /client.mjs` — the JS client, importable straight from a page.
pub async fn client_module() -> Response {
    serve(JS, CLIENT_MJS)
}

/// `GET /example/` — the runnable console.
pub async fn example_index() -> Response {
    serve(HTML, EXAMPLE_INDEX)
}

/// `GET /example/{*path}`.
pub async fn example_asset(axum::extract::Path(path): axum::extract::Path<String>) -> Response {
    match example_file(path.trim_start_matches('/')) {
        Some((content_type, body)) => serve(content_type, body),
        None => not_found(&path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_asset_is_present_and_non_trivial() {
        assert!(DOCS_INDEX.contains("<div id=\"swagger-ui\">"));
        assert!(DOCS_INIT.contains("SwaggerUIBundle"));
        assert!(
            SWAGGER_JS.len() > 1_000_000,
            "swagger bundle looks truncated"
        );
        assert!(SWAGGER_CSS.len() > 100_000);
        assert!(SWAGGER_LICENSE.contains("Apache License"));
        assert!(CLIENT_MJS.contains("export class BridgeApiError"));
        assert!(EXAMPLE_INDEX.contains("Claude bridge console"));
        assert!(EXAMPLE_JS.contains("streamOperation"));
        assert!(EXAMPLE_CSS.contains("--accent"));
    }

    #[test]
    fn the_vendor_manifest_records_the_bundle_we_actually_ship() {
        let manifest: serde_json::Value = serde_json::from_str(SWAGGER_VENDOR).unwrap();
        assert_eq!(manifest["package"], "swagger-ui-dist");
        assert_eq!(manifest["license"], "Apache-2.0");
        assert!(manifest["files_sha256"]["swagger-ui-bundle.js"].is_string());
    }

    #[test]
    fn asset_lookup_covers_the_pages_and_rejects_everything_else() {
        assert!(docs_file("swagger-ui.css").is_some());
        assert!(docs_file("index.html").is_some());
        assert!(docs_file("").is_some());
        assert!(docs_file("../../../etc/passwd").is_none());
        assert!(docs_file("nope.js").is_none());

        assert!(example_file("app.js").is_some());
        assert!(example_file("style.css").is_some());
        assert!(example_file("secrets").is_none());
    }

    #[test]
    fn the_client_exports_everything_the_example_imports() {
        for name in [
            "BridgeApiError",
            "chat",
            "createThread",
            "getHealth",
            "imageFromFile",
            "interruptTurn",
            "listModels",
            "listRequests",
            "respondToRequest",
            "startTurn",
            "streamOperation",
            "withImages",
        ] {
            assert!(
                CLIENT_MJS.contains(&format!("export const {name}"))
                    || CLIENT_MJS.contains(&format!("export function {name}"))
                    || CLIENT_MJS.contains(&format!("export async function {name}"))
                    || CLIENT_MJS.contains(&format!("export class {name}")),
                "client.mjs does not export {name}, which example/app.js imports"
            );
            assert!(
                EXAMPLE_JS.contains(name),
                "example/app.js does not use {name}"
            );
        }
    }
}
