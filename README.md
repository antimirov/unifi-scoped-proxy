# UniFi Scoped Proxy 🛡️

A lightweight, high-performance **zero-trust permission proxy** for the Ubiquiti UniFi Network & Protect APIs, written in Rust.

It allows you to safely expose UniFi APIs to local automation tools (like Home Assistant, AI Agents, or third-party scripts) by injecting your master UniFi API key server-side while enforcing **Homey-style granular permission scopes** and blocking unauthorized actions.

---

## Key Features

* **Homey-Style Granular Permission Scopes**: Configure domain-level access (`Sites`, `Devices Read`, `Devices Control`, `Clients Read`, `Clients Control`, `WLAN`, `Protect`) via environment variables.
* **Least-Privilege Defaults**: Dangerous actions (restarting devices, blocking clients, adopting devices) default to `false` (OFF).
* **Header Sanitization**: Injects your master `X-API-KEY` server-side so proxy clients never see or handle master credentials.
* **Optional Proxy Auth Token**: Require client authentication (`X-Proxy-Token` or `Bearer Token`) for added security.
* **Native Healthcheck Endpoint**: Dedicated `/healthz` endpoint for Docker and Kubernetes health probes.
* **Ultra-Lightweight & Fast**: Built on Rust (`Axum` + `Tokio`) compiled to a minimal `~4MB` Docker container.

---

## Permission Scopes Reference

All scopes can be toggled via environment variables:

| Environment Variable | Default | Matches Method & Path | Description |
| :--- | :--- | :--- | :--- |
| `SCOPE_INFO_READ` | `true` | `GET /v1/info` | Controller version and application info |
| `SCOPE_SITES_READ` | `true` | `GET /v1/sites*` | List sites & site metadata |
| `SCOPE_DEVICES_READ` | `false` | `GET /v1/sites/{id}/devices*` | Read APs, switches, gateways, & port states |
| `SCOPE_DEVICES_CONTROL`| `false` | `POST /v1/sites/{id}/devices/*/actions` | Execute actions (e.g. `RESTART`, `POWER_CYCLE`) |
| `SCOPE_DEVICES_ADOPT` | `false` | `POST /v1/sites/{id}/devices` | Adopt pending devices |
| `SCOPE_CLIENTS_READ` | `false` | `GET /v1/sites/{id}/clients*` | List connected Wi-Fi & LAN clients |
| `SCOPE_CLIENTS_CONTROL`| `false` | `POST /v1/sites/{id}/clients/*/actions` | Execute client actions (`BLOCK`, `UNBLOCK`, `RECONNECT`) |
| `SCOPE_WLAN_READ` | `false` | `GET /v1/sites/{id}/wlans*` | Read Wi-Fi SSID configurations |
| `SCOPE_PROTECT_READ` | `false` | `GET /v1/cameras*`, `GET /v1/meta/*` | Read UniFi Protect camera & NVR telemetry |

---

## Quickstart with Docker

### Docker Run
```bash
docker run -d \
  --name unifi-scoped-proxy \
  --restart always \
  -p 127.0.0.1:8080:8080 \
  -e UNIFI_BASE_URL="https://192.168.1.1" \
  -e UNIFI_API_KEY="your_master_unifi_api_key_here" \
  -e ACCEPT_INVALID_CERTS="true" \
  -e SCOPE_DEVICES_READ="true" \
  -e SCOPE_CLIENTS_READ="true" \
  ghcr.io/antimirov/unifi-scoped-proxy:latest
```

### Docker Compose
```yaml
version: '3.8'

services:
  unifi-scoped-proxy:
    image: ghcr.io/antimirov/unifi-scoped-proxy:latest
    container_name: unifi-scoped-proxy
    restart: always
    ports:
      - "127.0.0.1:8080:8080"
    environment:
      - UNIFI_BASE_URL=https://192.168.1.1
      - UNIFI_API_KEY=your_master_unifi_api_key_here
      - ACCEPT_INVALID_CERTS=true
      - SCOPE_INFO_READ=true
      - SCOPE_SITES_READ=true
      - SCOPE_DEVICES_READ=true
      - SCOPE_CLIENTS_READ=true
      - SCOPE_DEVICES_CONTROL=false
      - SCOPE_CLIENTS_CONTROL=false
    healthcheck:
      test: ["CMD", "wget", "-q", "-O", "-", "http://127.0.0.1:8080/healthz"]
      interval: 30s
      timeout: 5s
      retries: 3
```

---

## Testing & Usage

### 1. Read Sites List
```bash
curl -i http://127.0.0.1:8080/proxy/network/integration/v1/sites
```

### 2. Scope Violation Example (403 Forbidden)
If a client attempts an action that is disabled (e.g. restarting a device when `SCOPE_DEVICES_CONTROL=false`):
```bash
curl -i -X POST http://127.0.0.1:8080/proxy/network/integration/v1/sites/default/devices/123/actions \
  -H "Content-Type: application/json" \
  -d '{"action":"RESTART"}'
```
**Response**:
```http
HTTP/1.1 403 Forbidden
content-type: application/json

{
  "error": "Forbidden",
  "message": "Scope 'SCOPE_DEVICES_CONTROL' is disabled on this proxy gateway.",
  "required_scope": "SCOPE_DEVICES_CONTROL"
}
```

---

## Building from Source

### Prerequisites
* Rust 1.75+
* Cargo

```bash
# Clone the repository
git clone https://github.com/antimirov/unifi-scoped-proxy.git
cd unifi-scoped-proxy

# Build for release
cargo build --release

# Run locally
UNIFI_API_KEY="your_api_key" cargo run --release
```

---

## License

[MIT](LICENSE) © [antimirov](https://github.com/antimirov)
