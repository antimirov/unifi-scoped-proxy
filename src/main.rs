use dotenvy::dotenv;
use std::env;
use axum::{
    body::Body,
    extract::State,
    http::{Method, StatusCode, Uri, HeaderMap},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    client: reqwest::Client,
    unifi_base_url: String,
    api_key: String,
}

#[tokio::main]
async fn main() {
    // Load .env file if present
    let _ = dotenv();

    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "unifi_scoped_proxy=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Read configuration
    let unifi_base_url = env::var("UNIFI_BASE_URL")
        .unwrap_or_else(|_| "https://192.168.1.1".to_string());
    let api_key = env::var("UNIFI_API_KEY")
        .expect("UNIFI_API_KEY environment variable is required");
    let listen_addr_str = env::var("LISTEN_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let accept_invalid_certs = env::var("ACCEPT_INVALID_CERTS")
        .map(|v| v.to_lowercase() == "true")
        .unwrap_or(false);

    tracing::info!("Starting UniFi Scoped Proxy...");
    tracing::info!("UniFi Base URL: {}", unifi_base_url);
    tracing::info!("Accept Invalid Certs: {}", accept_invalid_certs);

    // Build reqwest client
    let mut client_builder = reqwest::Client::builder();
    if accept_invalid_certs {
        client_builder = client_builder.danger_accept_invalid_certs(true);
    }
    let client = client_builder
        .build()
        .expect("Failed to build reqwest client");

    let state = AppState {
        client,
        unifi_base_url,
        api_key,
    };

    // Configure router
    let app = Router::new()
        .fallback(any(proxy_handler))
        .with_state(state)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&listen_addr_str)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind to {}: {}", listen_addr_str, e));
    tracing::info!("Listening on http://{}", listener.local_addr().unwrap());

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

async fn proxy_handler(
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    // 1. Enforce GET-only (Read-Only) restriction
    if method != Method::GET {
        tracing::warn!("Blocked non-GET request: {} {}", method, uri.path());
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            "Method Not Allowed. This proxy is read-only and only GET requests are permitted.",
        )
            .into_response();
    }

    // 2. Reconstruct target URL
    let target_url = if let Some(query) = uri.query() {
        format!("{}{}?{}", state.unifi_base_url, uri.path(), query)
    } else {
        format!("{}{}", state.unifi_base_url, uri.path())
    };

    tracing::info!("Proxying GET request to: {}", target_url);

    // 3. Prepare outgoing request
    let mut req_builder = state.client.request(reqwest::Method::GET, &target_url);

    // Forward original headers, except Host
    for (key, value) in headers.iter() {
        if key != reqwest::header::HOST {
            req_builder = req_builder.header(key, value);
        }
    }

    // Inject UniFi API Key
    req_builder = req_builder.header("X-API-KEY", &state.api_key);

    // 4. Send request
    let res = match req_builder.send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to forward request to UniFi controller: {:?}", e);
            return (
                StatusCode::BAD_GATEWAY,
                "Failed to connect to the UniFi Controller.",
            )
                .into_response();
        }
    };

    let status = res.status();
    let mut response_builder = Response::builder().status(status.as_u16());

    // 5. Forward essential headers from UniFi response
    if let Some(content_type) = res.headers().get(reqwest::header::CONTENT_TYPE) {
        response_builder = response_builder.header(reqwest::header::CONTENT_TYPE, content_type);
    }

    // 6. Read and return the body
    match res.bytes().await {
        Ok(body_bytes) => {
            response_builder
                .body(Body::from(body_bytes))
                .unwrap_or_else(|e| {
                    tracing::error!("Failed to build response: {:?}", e);
                    (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
                })
        }
        Err(e) => {
            tracing::error!("Failed to read response body from UniFi: {:?}", e);
            (
                StatusCode::BAD_GATEWAY,
                "Failed to read response from the UniFi Controller.",
            )
                .into_response()
        }
    }
}
