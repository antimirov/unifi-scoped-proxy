use dotenvy::dotenv;
use std::env;
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use subtle::ConstantTimeEq;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone, Debug)]
struct PermissionScopes {
    info_read: bool,
    sites_read: bool,
    devices_read: bool,
    devices_control: bool,
    devices_adopt: bool,
    clients_read: bool,
    clients_control: bool,
    wlan_read: bool,
    protect_read: bool,
}

impl PermissionScopes {
    fn from_env() -> Self {
        Self {
            info_read: env_bool("SCOPE_INFO_READ", true),
            sites_read: env_bool("SCOPE_SITES_READ", true),
            devices_read: env_bool("SCOPE_DEVICES_READ", false),
            devices_control: env_bool("SCOPE_DEVICES_CONTROL", false),
            devices_adopt: env_bool("SCOPE_DEVICES_ADOPT", false),
            clients_read: env_bool("SCOPE_CLIENTS_READ", false),
            clients_control: env_bool("SCOPE_CLIENTS_CONTROL", false),
            wlan_read: env_bool("SCOPE_WLAN_READ", false),
            protect_read: env_bool("SCOPE_PROTECT_READ", false),
        }
    }
}

fn env_bool(var_name: &str, default_value: bool) -> bool {
    env::var(var_name)
        .map(|v| match v.to_lowercase().as_str() {
            "true" | "1" | "yes" => true,
            "false" | "0" | "no" => false,
            _ => default_value,
        })
        .unwrap_or(default_value)
}

#[derive(Clone)]
struct AppState {
    client: reqwest::Client,
    unifi_base_url: String,
    api_key: String,
    scopes: PermissionScopes,
    proxy_auth_token: Option<String>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("unifi_base_url", &self.unifi_base_url)
            .field("api_key", &"[REDACTED]")
            .field("proxy_auth_token", &self.proxy_auth_token.as_ref().map(|_| "[REDACTED]"))
            .field("scopes", &self.scopes)
            .finish()
    }
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
    let accept_invalid_certs = env_bool("ACCEPT_INVALID_CERTS", false);
    let proxy_auth_token = env::var("PROXY_AUTH_TOKEN").ok().filter(|s| !s.is_empty());

    let scopes = PermissionScopes::from_env();

    tracing::info!("Starting UniFi Scoped Proxy...");
    tracing::info!("UniFi Base URL: {}", unifi_base_url);
    tracing::info!("Accept Invalid Certs: {}", accept_invalid_certs);
    if proxy_auth_token.is_some() {
        tracing::info!("Proxy Auth Token: ENFORCED");
    } else {
        tracing::info!("Proxy Auth Token: Disabled (open local access)");
    }
    tracing::info!("Active Permission Scopes: {:?}", scopes);

    // Build reqwest client
    let mut client_builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(30));
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
        scopes,
        proxy_auth_token,
    };

    // Configure router
    let app = Router::new()
        .fallback(any(proxy_handler))
        .with_state(state)
        .layer(DefaultBodyLimit::max(1024 * 1024)) // 1 MB max request body
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
    body: axum::body::Bytes,
) -> Response {
    let path = uri.path();

    // 0. Native Healthcheck Endpoint
    if path == "/healthz" || path == "/health" {
        return (
            StatusCode::OK,
            [("content-type", "application/json")],
            r#"{"status":"ok"}"#,
        )
            .into_response();
    }

    // 1. Client Proxy Authentication check (if enabled)
    if let Some(expected_token) = &state.proxy_auth_token {
        let provided_token = headers
            .get("X-Proxy-Token")
            .and_then(|v| v.to_str().ok())
            .or_else(|| {
                headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.strip_prefix("Bearer "))
            });

        // Constant-time comparison to prevent timing side-channel attacks
        let is_valid = provided_token
            .map(|t| {
                let t_bytes = t.as_bytes();
                let e_bytes = expected_token.as_bytes();
                t_bytes.len() == e_bytes.len() && bool::from(t_bytes.ct_eq(e_bytes))
            })
            .unwrap_or(false);

        if !is_valid {
            tracing::warn!("Unauthorized proxy access attempt to path: {}", path);
            return (
                StatusCode::UNAUTHORIZED,
                [("content-type", "application/json")],
                r#"{"error":"Unauthorized","message":"Invalid or missing proxy authentication token."}"#,
            )
                .into_response();
        }
    }

    // 2. Validate Granular Permission Scope
    if let Err((status, error_json)) = check_scope_permission(&method, path, &state.scopes) {
        tracing::warn!("Scope violation: {} {} -> {}", method, path, error_json);
        return (
            status,
            [("content-type", "application/json")],
            error_json,
        )
            .into_response();
    }

    // 3. Reconstruct target URL
    let target_url = if let Some(query) = uri.query() {
        format!("{}{}?{}", state.unifi_base_url, path, query)
    } else {
        format!("{}{}", state.unifi_base_url, path)
    };

    tracing::info!("Proxying {} request to: {}", method, target_url);

    // 4. Prepare outgoing request
    let reqwest_method = match reqwest::Method::from_bytes(method.as_str().as_bytes()) {
        Ok(m) => m,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                [("content-type", "application/json")],
                r#"{"error":"Bad Request","message":"Invalid HTTP method."}"#,
            )
                .into_response()
        }
    };

    let mut req_builder = state.client.request(reqwest_method, &target_url);

    // Forward original headers, stripping security-sensitive ones
    const STRIP_HEADERS: [&str; 6] = [
        "host", "authorization", "x-proxy-token",
        "cookie", "x-forwarded-for", "x-real-ip",
    ];
    for (key, value) in headers.iter() {
        if !STRIP_HEADERS.contains(&key.as_str()) {
            req_builder = req_builder.header(key, value);
        }
    }

    // Inject UniFi API Key
    req_builder = req_builder.header("X-API-KEY", &state.api_key);

    // Forward the request body
    req_builder = req_builder.body(body);

    // 5. Send request
    let res = match req_builder.send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to forward request to UniFi controller: {:?}", e);
            return (
                StatusCode::BAD_GATEWAY,
                [("content-type", "application/json")],
                r#"{"error":"Bad Gateway","message":"Failed to connect to the UniFi Controller."}"#,
            )
                .into_response();
        }
    };

    let status = res.status();
    let mut response_builder = Response::builder().status(status.as_u16());

    // 6. Forward response headers (strip only hop-by-hop headers)
    for (key, value) in res.headers().iter() {
        if key != "transfer-encoding" && key != "connection" {
            response_builder = response_builder.header(key, value);
        }
    }

    // 7. Read and return the body
    match res.bytes().await {
        Ok(body_bytes) => response_builder
            .body(Body::from(body_bytes))
            .unwrap_or_else(|e| {
                tracing::error!("Failed to build response: {:?}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [("content-type", "application/json")],
                    r#"{"error":"Internal Error","message":"Failed to build response."}"#,
                )
                    .into_response()
            }),
        Err(e) => {
            tracing::error!("Failed to read response body from UniFi: {:?}", e);
            (
                StatusCode::BAD_GATEWAY,
                [("content-type", "application/json")],
                r#"{"error":"Bad Gateway","message":"Failed to read response from UniFi Controller."}"#,
            )
                .into_response()
        }
    }
}

fn check_scope_permission(
    method: &Method,
    raw_path: &str,
    scopes: &PermissionScopes,
) -> Result<(), (StatusCode, String)> {
    // Normalize path by stripping known proxy prefixes
    let path = raw_path
        .strip_prefix("/proxy/network/integration")
        .or_else(|| raw_path.strip_prefix("/proxy/network"))
        .unwrap_or(raw_path);

    // Helper to format forbidden response JSON
    let forbidden = |scope_var: &str| {
        (
            StatusCode::FORBIDDEN,
            format!(
                r#"{{"error":"Forbidden","message":"Scope '{scope_var}' is disabled on this proxy gateway.","required_scope":"{scope_var}"}}"#
            ),
        )
    };

    // Helper for Method Not Allowed
    let method_not_allowed = || {
        (
            StatusCode::METHOD_NOT_ALLOWED,
            r#"{"error":"Method Not Allowed","message":"This HTTP method is not permitted for the requested endpoint."}"#.to_string(),
        )
    };

    // System Info
    if path == "/v1/info" {
        if method == Method::GET {
            if scopes.info_read {
                return Ok(());
            } else {
                return Err(forbidden("SCOPE_INFO_READ"));
            }
        } else {
            return Err(method_not_allowed());
        }
    }

    // Sites
    if path == "/v1/sites"
        || path.starts_with("/v1/sites/") && path.trim_start_matches("/v1/sites/").find('/').is_none()
    {
        if method == Method::GET {
            if scopes.sites_read {
                return Ok(());
            } else {
                return Err(forbidden("SCOPE_SITES_READ"));
            }
        } else {
            return Err(method_not_allowed());
        }
    }

    // Devices
    if path.contains("/devices") {
        if path.ends_with("/actions") {
            if method == Method::POST {
                if scopes.devices_control {
                    return Ok(());
                } else {
                    return Err(forbidden("SCOPE_DEVICES_CONTROL"));
                }
            } else {
                return Err(method_not_allowed());
            }
        } else if path.ends_with("/devices") && method == Method::POST {
            if scopes.devices_adopt {
                return Ok(());
            } else {
                return Err(forbidden("SCOPE_DEVICES_ADOPT"));
            }
        } else if method == Method::GET {
            if scopes.devices_read {
                return Ok(());
            } else {
                return Err(forbidden("SCOPE_DEVICES_READ"));
            }
        } else {
            return Err(method_not_allowed());
        }
    }

    // Clients
    if path.contains("/clients") {
        if path.ends_with("/actions") {
            if method == Method::POST {
                if scopes.clients_control {
                    return Ok(());
                } else {
                    return Err(forbidden("SCOPE_CLIENTS_CONTROL"));
                }
            } else {
                return Err(method_not_allowed());
            }
        } else if method == Method::GET {
            if scopes.clients_read {
                return Ok(());
            } else {
                return Err(forbidden("SCOPE_CLIENTS_READ"));
            }
        } else {
            return Err(method_not_allowed());
        }
    }

    // WLAN / Wi-Fi
    if path.contains("/wlans") || path.contains("/wifi-broadcasts") {
        if method == Method::GET {
            if scopes.wlan_read {
                return Ok(());
            } else {
                return Err(forbidden("SCOPE_WLAN_READ"));
            }
        } else {
            return Err(method_not_allowed());
        }
    }

    // Protect Cameras / NVR
    if path.contains("/protect") || path.contains("/cameras") {
        if method == Method::GET {
            if scopes.protect_read {
                return Ok(());
            } else {
                return Err(forbidden("SCOPE_PROTECT_READ"));
            }
        } else {
            return Err(method_not_allowed());
        }
    }

    // Fallback: deny by default — only explicitly matched paths are allowed
    Err((
        StatusCode::FORBIDDEN,
        r#"{"error":"Forbidden","message":"No scope rule matches this endpoint. Access denied by default."}"#.to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_scopes(
        info: bool,
        sites: bool,
        dev_read: bool,
        dev_ctrl: bool,
        dev_adopt: bool,
        cli_read: bool,
        cli_ctrl: bool,
        wlan: bool,
        protect: bool,
    ) -> PermissionScopes {
        PermissionScopes {
            info_read: info,
            sites_read: sites,
            devices_read: dev_read,
            devices_control: dev_ctrl,
            devices_adopt: dev_adopt,
            clients_read: cli_read,
            clients_control: cli_ctrl,
            wlan_read: wlan,
            protect_read: protect,
        }
    }

    #[test]
    fn test_info_read_scope() {
        let scopes = test_scopes(true, false, false, false, false, false, false, false, false);
        assert!(check_scope_permission(&Method::GET, "/v1/info", &scopes).is_ok());

        let scopes_disabled = test_scopes(false, false, false, false, false, false, false, false, false);
        assert!(check_scope_permission(&Method::GET, "/v1/info", &scopes_disabled).is_err());
        assert!(check_scope_permission(&Method::POST, "/v1/info", &scopes).is_err());
    }

    #[test]
    fn test_sites_read_scope() {
        let scopes = test_scopes(false, true, false, false, false, false, false, false, false);
        assert!(check_scope_permission(&Method::GET, "/v1/sites", &scopes).is_ok());
        assert!(check_scope_permission(&Method::GET, "/proxy/network/v1/sites", &scopes).is_ok());
        assert!(check_scope_permission(&Method::GET, "/v1/sites/default", &scopes).is_ok());

        let scopes_disabled = test_scopes(false, false, false, false, false, false, false, false, false);
        assert!(check_scope_permission(&Method::GET, "/v1/sites", &scopes_disabled).is_err());
        assert!(check_scope_permission(&Method::POST, "/v1/sites", &scopes).is_err());
    }

    #[test]
    fn test_devices_scopes() {
        // Read test
        let scopes_read = test_scopes(false, false, true, false, false, false, false, false, false);
        assert!(check_scope_permission(&Method::GET, "/v1/sites/default/devices", &scopes_read).is_ok());
        assert!(check_scope_permission(&Method::GET, "/v1/sites/default/devices/dev123", &scopes_read).is_ok());
        assert!(check_scope_permission(&Method::POST, "/v1/sites/default/devices/dev123/actions", &scopes_read).is_err());

        // Control test
        let scopes_ctrl = test_scopes(false, false, false, true, false, false, false, false, false);
        assert!(check_scope_permission(&Method::POST, "/v1/sites/default/devices/dev123/actions", &scopes_ctrl).is_ok());
        assert!(check_scope_permission(&Method::GET, "/v1/sites/default/devices", &scopes_ctrl).is_err());

        // Adopt test
        let scopes_adopt = test_scopes(false, false, false, false, true, false, false, false, false);
        assert!(check_scope_permission(&Method::POST, "/v1/sites/default/devices", &scopes_adopt).is_ok());
    }

    #[test]
    fn test_clients_scopes() {
        // Read test
        let scopes_read = test_scopes(false, false, false, false, false, true, false, false, false);
        assert!(check_scope_permission(&Method::GET, "/v1/sites/default/clients", &scopes_read).is_ok());
        assert!(check_scope_permission(&Method::POST, "/v1/sites/default/clients/cli123/actions", &scopes_read).is_err());

        // Control test
        let scopes_ctrl = test_scopes(false, false, false, false, false, false, true, false, false);
        assert!(check_scope_permission(&Method::POST, "/v1/sites/default/clients/cli123/actions", &scopes_ctrl).is_ok());
        assert!(check_scope_permission(&Method::GET, "/v1/sites/default/clients", &scopes_ctrl).is_err());
    }

    #[test]
    fn test_wlan_and_protect_scopes() {
        let scopes_wlan = test_scopes(false, false, false, false, false, false, false, true, false);
        assert!(check_scope_permission(&Method::GET, "/v1/sites/default/wlans", &scopes_wlan).is_ok());

        let scopes_protect = test_scopes(false, false, false, false, false, false, false, false, true);
        assert!(check_scope_permission(&Method::GET, "/v1/cameras", &scopes_protect).is_ok());
        assert!(check_scope_permission(&Method::GET, "/proxy/network/v1/sites", &scopes_protect).is_err());
    }

    #[test]
    fn test_deny_by_default_on_unknown_paths() {
        // All scopes enabled — unknown paths should still be denied
        let scopes_all = test_scopes(true, true, true, true, true, true, true, true, true);
        assert!(check_scope_permission(&Method::GET, "/v1/some/future/endpoint", &scopes_all).is_err());
        assert!(check_scope_permission(&Method::GET, "/v2/new-api", &scopes_all).is_err());
        assert!(check_scope_permission(&Method::POST, "/v1/admin/settings", &scopes_all).is_err());
    }

    #[test]
    fn test_normalized_proxy_integration_prefix() {
        let scopes = test_scopes(true, true, true, false, false, true, false, false, false);

        // /proxy/network/integration/ prefix → normalized to /v1/...
        assert!(check_scope_permission(&Method::GET, "/proxy/network/integration/v1/sites", &scopes).is_ok());
        assert!(check_scope_permission(&Method::GET, "/proxy/network/integration/v1/info", &scopes).is_ok());
        assert!(check_scope_permission(&Method::GET, "/proxy/network/integration/v1/sites/default/clients", &scopes).is_ok());
        assert!(check_scope_permission(&Method::GET, "/proxy/network/integration/v1/sites/default/devices", &scopes).is_ok());

        // /proxy/network/ prefix → also normalized
        assert!(check_scope_permission(&Method::GET, "/proxy/network/v1/sites", &scopes).is_ok());
        assert!(check_scope_permission(&Method::GET, "/proxy/network/v1/info", &scopes).is_ok());

        // Real-world UUID site ID path
        assert!(check_scope_permission(
            &Method::GET,
            "/proxy/network/integration/v1/sites/88f7af54-98f8-306a-a1c7-c9349722b1f6/clients",
            &scopes
        ).is_ok());
    }
}
