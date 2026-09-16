use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::{header::HOST, HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
    response::IntoResponse,
    routing::any,
    Router,
};
use bytes::Bytes;
use futures_util::TryStreamExt;
use std::net::SocketAddr;

const PRODUCT_HEADER: &str = "x-elastic-product";
const PRODUCT_VALUE: &str = "Elasticsearch";

#[derive(Clone)]
struct AppState {
    upstream: String,
    fake_version: String,
    client: reqwest::Client,
}

#[tokio::main]
async fn main() {
    let upstream = std::env::var("UPSTREAM").unwrap_or_else(|_| "http://127.0.0.1:9200".into());
    let listen = std::env::var("LISTEN").unwrap_or_else(|_| "0.0.0.0:9201".into());
    let fake_version = std::env::var("FAKE_ES_VERSION").unwrap_or_else(|_| "7.10.2".into());

    let state = AppState {
        upstream: upstream.trim_end_matches('/').to_string(),
        fake_version,
        client: reqwest::Client::new(),
    };

    eprintln!(
        "opensearch-compat-proxy: listening on {listen}, forwarding to {} (spoofing version {})",
        state.upstream, state.fake_version
    );

    let app = Router::new().fallback(any(proxy)).with_state(state);

    let addr: SocketAddr = listen.parse().expect("invalid LISTEN address");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind LISTEN address");
    axum::serve(listener, app).await.expect("server error");
}

async fn proxy(State(state): State<AppState>, req: Request) -> axum::response::Response {
    let method = req.method().clone();
    let is_root_get = method == Method::GET && req.uri().path() == "/";
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/");
    let target_url = format!("{}{}", state.upstream, path_and_query);

    let mut out_headers = reqwest::header::HeaderMap::new();
    for (name, value) in req.headers().iter() {
        // Let reqwest derive its own Host header for the upstream request.
        if name == HOST {
            continue;
        }
        if let (Ok(n), Ok(v)) = (
            reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()),
            reqwest::header::HeaderValue::from_bytes(value.as_bytes()),
        ) {
            out_headers.append(n, v);
        }
    }

    // NOTE: request bodies are buffered in full before being forwarded (simplest correct
    // implementation for a compatibility shim). If you need to proxy very large bulk
    // payloads without buffering them in memory, switch this to a streamed reqwest::Body
    // built from req.into_body().into_data_stream().
    let body_bytes = match to_bytes(req.into_body(), usize::MAX).await {
        Ok(b) => b,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("failed to read request body: {err}"),
            )
                .into_response();
        }
    };

    let mut upstream_req = state
        .client
        .request(reqwest_method(&method), &target_url)
        .headers(out_headers);
    if !body_bytes.is_empty() {
        upstream_req = upstream_req.body(body_bytes);
    }

    let upstream_resp = match upstream_req.send().await {
        Ok(r) => r,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("upstream request failed: {err}"),
            )
                .into_response();
        }
    };

    let status =
        StatusCode::from_u16(upstream_resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);

    let mut resp_headers = HeaderMap::new();
    for (name, value) in upstream_resp.headers().iter() {
        // Let axum/hyper recompute framing headers for the outgoing response.
        if name == reqwest::header::TRANSFER_ENCODING || name == reqwest::header::CONTENT_LENGTH {
            continue;
        }
        if let (Ok(n), Ok(v)) = (
            HeaderName::from_bytes(name.as_str().as_bytes()),
            HeaderValue::from_bytes(value.as_bytes()),
        ) {
            resp_headers.append(n, v);
        }
    }
    // The actual fix: recent Elastic clients refuse to talk to a server that doesn't send
    // this header, and OpenSearch never sends it.
    resp_headers.insert(
        HeaderName::from_static(PRODUCT_HEADER),
        HeaderValue::from_static(PRODUCT_VALUE),
    );

    let body: Body = if is_root_get {
        // The root `GET /` response is where clients read `version.number` to decide
        // whether they support talking to this server at all - rewrite it to the last
        // Apache-2.0 Elasticsearch release OpenSearch stayed wire-compatible with.
        match upstream_resp.bytes().await {
            Ok(bytes) => {
                let rewritten = rewrite_root_response(&bytes, &state.fake_version)
                    .unwrap_or_else(|| bytes.to_vec());
                Body::from(rewritten)
            }
            Err(err) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    format!("failed to read upstream body: {err}"),
                )
                    .into_response();
            }
        }
    } else {
        let stream = upstream_resp.bytes_stream().map_err(std::io::Error::other);
        Body::from_stream(stream)
    };

    (status, resp_headers, body).into_response()
}

fn rewrite_root_response(original: &Bytes, fake_version: &str) -> Option<Vec<u8>> {
    let mut json: serde_json::Value = serde_json::from_slice(original).ok()?;
    json.get_mut("version")?
        .as_object_mut()?
        .insert(
            "number".to_string(),
            serde_json::Value::String(fake_version.to_string()),
        );
    serde_json::to_vec(&json).ok()
}

fn reqwest_method(method: &Method) -> reqwest::Method {
    reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET)
}
