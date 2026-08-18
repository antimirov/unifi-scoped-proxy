# UniFi Scoped Proxy 🛡️

[![CI](https://img.shields.io/github/actions/workflow/status/antimirov/unifi-scoped-proxy/docker-publish.yml?branch=main&style=flat-square)](https://github.com/antimirov/unifi-scoped-proxy/actions)
[![GitHub Release](https://img.shields.io/github/v/release/antimirov/unifi-scoped-proxy?style=flat-square)](https://github.com/antimirov/unifi-scoped-proxy/releases)
[![Docker Image Size](https://img.shields.io/badge/docker%20image-~4MB-brightgreen?style=flat-square)](https://ghcr.io/antimirov/unifi-scoped-proxy)
[![Rust 1.75+](https://img.shields.io/badge/rust-1.75%2B-orange.svg?style=flat-square)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg?style=flat-square)](LICENSE)

> **A lightweight zero-trust scoped permission proxy for the Ubiquiti UniFi Network & Protect APIs.**  
> Turn your full-access UniFi API keys into restricted, read-only endpoints with granular permission scopes for Home Assistant, AI agents, and homelab automations.

---

## Why UniFi Scoped Proxy?

Official Ubiquiti UniFi API keys are **all-or-nothing**: they grant full administrative permissions across your entire network. There is currently no native way in UniFi OS to create a **readonly API key** or restrict an API token to specific actions or sites.

If you integrate an external tool (such as Home Assistant, an autonomous AI Agent like Hermes, a Prometheus exporter, or a custom script), sharing your master API key exposes your network to substantial risks:
* **Accidental or malicious write actions**: Disconnecting clients, power-cycling PoE ports, restarting gateways, or adopting rogue devices.
* **Credential leakage**: Third-party services or logs having access to your root master UniFi key.
* **Scope sprawl**: Tools requesting simple device counts gaining access to Wi-Fi credentials or camera configurations.

**UniFi Scoped Proxy** solves this by acting as a zero-trust gateway. It stores your master `X-API-KEY` server-side, intercepts every incoming HTTP request, normalizes the API path, and strictly evaluates it against configurable **least-privilege permission scopes** before forwarding it to your UniFi Gateway.

---

## Common Use Cases

* **Read-Only Home Assistant & Dashboard Integrations**: Safely query site status, device lists, and client connections without risking destructive write operations.
* **Safe LLM & AI Agent Tooling (e.g. Hermes Agent)**: Give autonomous agents visibility into your network topology to answer queries, while physically preventing them from modifying SSIDs or rebooting hardware.
* **Homelab & Multi-Container Isolation**: Run the proxy alongside other Docker containers on an isolated bridge network, exposing only the exact endpoints each service requires with zero exposed host ports.
* **Readonly UniFi Protect Camera Feeds**: Allow monitoring scripts to inspect camera metadata while blocking management operations.

---

## Key Features

* 🔒 **Granular Permission Scopes & Readonly Mode**: Toggle fine-grained access per domain (`Sites`, `Devices Read`, `Devices Control`, `Clients Read`, `Clients Control`, `WLAN`, `Protect`) via simple environment variables.
* 🛑 **Zero-Trust & Deny-by-Default**: Unknown or newly introduced UniFi API endpoints are blocked by default (`403 Forbidden`) unless explicitly permitted by an enabled scope.
* 🛡️ **Server-Side Credential Injection**: Master `X-API-KEY` is injected securely by the proxy—clients never see, store, or transmit root credentials.
* 🧹 **Header Sanitization**: Strips sensitive client headers (`Authorization`, `X-Proxy-Token`, `Cookie`, `X-Forwarded-For`) before forwarding requests upstream.
* 🔑 **Optional Client Authentication**: Require a dedicated `X-Proxy-Token` or `Bearer` token for proxy clients, validated with constant-time equality checks.
* ⏱️ **Resilient Network Operations**: Built-in 5s connection timeout, 30s request timeout, and 1 MB request body limit to protect upstream controllers.
* 🩺 **Native Healthcheck Endpoint**: Dedicated `/healthz` and `/health` endpoints for Docker and Kubernetes liveness probes.
* ⚡ **Ultra-Lightweight & Blazing Fast**: Built in Rust with `Axum` and `Tokio`, packaged in a minimal `~4MB` scratch-like Alpine container with multi-arch support (`amd64` and `arm64`).

---

## Permission Scopes Reference

All scopes can be individually toggled via environment variables. Sensitive control actions default to `false` (disabled):

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
| `SCOPE_PROTECT_READ` | `false` | `GET /v1/cameras*`, `GET /v1/meta/*` | Read UniFi Protect camera & NVR data |

---

## Quickstart with Docker

### 1. Docker CLI
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

### 2. Docker Compose
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

### 3. Shared Docker Network (Zero Host Port Exposure)
When deploying alongside an AI agent (such as `hermes-agent`) or Home Assistant on the same machine or Synology NAS Container Manager, attach both containers to an internal bridge network. This allows the agent to communicate with the proxy over Docker DNS with **zero ports exposed to your local network**:

```yaml
networks:
  agent-net:
    driver: bridge

services:
  hermes:
    image: nousresearch/hermes-agent:latest
    container_name: hermes-agent
    restart: always
    ports:
      - "8642:8642"
    networks:
      - agent-net
    volumes:
      - /volume1/docker/hermes:/opt/data
    command: gateway run

  unifi-scoped-proxy:
    image: ghcr.io/antimirov/unifi-scoped-proxy:latest
    container_name: unifi-scoped-proxy
    restart: always
    networks:
      - agent-net
    environment:
      - UNIFI_BASE_URL=https://192.168.1.1
      - UNIFI_API_KEY=your_master_unifi_api_key_here
      - ACCEPT_INVALID_CERTS=true
      - SCOPE_INFO_READ=true
      - SCOPE_SITES_READ=true
      - SCOPE_DEVICES_READ=true
      - SCOPE_CLIENTS_READ=true
```
* **Endpoint for AI Agent**: `http://unifi-scoped-proxy:8080/proxy/network/integration/v1/sites`

---

## Testing & Usage

### 1. Read Sites (Permitted)
```bash
curl -i http://127.0.0.1:8080/proxy/network/integration/v1/sites
```

### 2. Scope Violation Example (403 Forbidden)
If an integration attempts a control action that has been disabled in the environment (e.g. `SCOPE_DEVICES_CONTROL=false`):
```bash
curl -i -X POST http://127.0.0.1:8080/proxy/network/integration/v1/sites/default/devices/dev_123/actions \
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

### 3. Health Probe Check
```bash
curl -i http://127.0.0.1:8080/healthz
```
**Response**:
```http
HTTP/1.1 200 OK
content-type: application/json

{"status":"ok"}
```

---

## Building from Source

### Prerequisites
* [Rust 1.75+](https://www.rust-lang.org/)
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

## Docker & CI/CD Pipeline

* **Multi-Stage Dockerfile**: Uses a lightweight Alpine build stage to compile the Rust binary, producing a minimal `~4MB` runtime container.
* **Fast `amd64` Builds on Push**: Everyday pushes to `main` compile and update `linux/amd64` in ~45 seconds.
* **Multi-Arch Releases (`amd64` + `arm64`)**: Creating a version release tag (e.g. `v0.1.0`) triggers a full multi-architecture build for x86 and Apple Silicon / ARM devices.
* **Dynamic Manifest Stitching**: Pushes to `main` dynamically stitch the new `amd64` build with the active release `arm64` digest, ensuring `ghcr.io/antimirov/unifi-scoped-proxy:latest` always resolves natively for both x86 and ARM platforms.

---

## License

[MIT](LICENSE) © [antimirov](https://github.com/antimirov)
