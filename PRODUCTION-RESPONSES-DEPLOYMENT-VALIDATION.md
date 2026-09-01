# Production Responses Deployment Validation

- Date: 2026-08-06
- Workspace: `/root/projects/codewhale-proxy/source`
- Branch: `feat/gpt-responses-transport`
- Service: `cc-proxy.service`
- Production endpoint: `127.0.0.1:11441`
- Temporary endpoint: not used; no listener observed on `11449`
- Credentials and request content: not recorded; validation used an in-memory placeholder Authorization value and sanitized summaries only.

## 1. Pre-deployment gates

Live checks completed before deployment:

```text
branch: feat/gpt-responses-transport
production service: active/running
production listener: 0.0.0.0:11441
clawbot resolution: 100.64.0.1
11441 health: HTTP 200, {"service":"cc-proxy","status":"ok"}
```

Repository quality gates:

```text
cargo test --all-targets --locked: PASS (82 passed, 0 failed)
cargo fmt --all -- --check: PASS
cargo clippy --all-targets --all-features -- -D warnings: PASS
git diff --check: PASS (pre-deployment check)
```

Release artifact:

```text
target/release/cc-proxy: built successfully from the current workspace
release SHA-256: 8ba17db4e80ff5e349f5f3faa01fb57eefe14e388220a8246f7b604e88afbe4c
```

## 2. Backup and minimal production configuration

Backup was created before stopping the service:

```text
backup directory: /var/backups/cc-proxy/20260806-103640
backup files: config.toml, cc-proxy
```

The attempted minimal configuration contained only the requested mapping change and the Responses wire setting:

```text
claude-sonnet-4-6 -> gpt-5.6-luna
[gpt-5.6-luna] wire_api = "responses"
```

DeepSeek, GLM, Kimi, other mappings, and providers were retained in the attempted configuration. No secret values are included in this report.

## 3. Deployment and rollback

The requested sequence was executed:

```text
systemctl stop cc-proxy.service
replace /usr/local/bin/cc-proxy
replace /etc/cc-proxy/config.toml
systemctl start cc-proxy.service
```

After the replacement, service startup succeeded and reported five model profiles/four providers, listener `0.0.0.0:11441`, upstream `http://clawbot:11434`, and an upstream health check of OK. The post-start health endpoint returned HTTP 200.

Production business validation then failed the required gates:

```text
DeepSeek Chat: HTTP 200 (usage observed), but stop_reason=max_tokens for the small probe
GLM Chat: HTTP 200 (usage observed)
gpt-5.6 Responses non-stream: HTTP 200 (usage observed)
Responses SSE: HTTP 200, text/event-stream, message_start/message_stop observed
Tool continuation 1: HTTP 200 initial response but no real tool_use ID; continuation not valid
Tool continuation 2: HTTP 200 initial response but no real tool_use ID; continuation not valid
```

The tool continuation requirement therefore failed. In accordance with the safety gate, the service was stopped and both production files were restored from the backup.

Rollback result:

```text
rollback binary SHA-256: 42689803b3d9f26a20e7096fe55aeaf716dbf4684ac6551afde3fc0c40ddab7f
backup binary SHA-256:   42689803b3d9f26a20e7096fe55aeaf716dbf4684ac6551afde3fc0c40ddab7f
service: active/running
11441 listener: present
final /health: HTTP 200, {"service":"cc-proxy","status":"ok"}
11449 listener: absent
```

The final production mapping is restored to `claude-sonnet-4-6 -> deepseek-v4-pro`; the restored production profile does not enable `wire_api = "responses"` for `gpt-5.6-luna`.

## 4. Final decision

**ROLLED_BACK / NOT APPROVED FOR PRODUCTION.** The release artifact and startup/health checks were valid, but the required real tool continuation acceptance gate was not met. Production is active on the backed-up binary and configuration. The failure is recorded as a business/protocol validation failure, not as a health-check success or a cache conclusion.

The attempted live probe observed `cache_read_input_tokens=0` on the short Responses samples; these samples were not counted as the required long-prefix cache evidence. No claim is made about the long-prefix cache rate from this deployment attempt. Historical timeout/502 observations remain separate failure categories.


## 5. Main-agent revalidation after rollback (2026-08-06 11:00 CST)

The previous failed deployment was rolled back. The main agent then performed a second controlled deployment using the verified release artifact and a newly verified minimal production configuration diff.

### 5.1 Deployment facts

```text
backup directory: /var/backups/cc-proxy/manual-20260806-105806
release artifact: /root/projects/codewhale-proxy/source/target/release/cc-proxy
release SHA-256: 5b856b7c41fd29d3345deb07810f24a2d27bbb8ed1859a510f84a61d979ac9f9
production binary after replacement: same SHA-256
production config after replacement: ba5659ddb6eb75e229c0f68bccd5486c2831e12a6be9d97fabd20364f65fa34c
```

The production configuration change was limited to:

```text
claude-sonnet-4-6 -> gpt-5.6-luna
[gpt-5.6-luna] wire_api = "responses"
```

The parsed configuration retained five profiles and four providers. The previous production binary/configuration were backed up and checksum-verified before replacement.

Deployment sequence:

```text
systemctl stop cc-proxy.service
replace /usr/local/bin/cc-proxy
replace /etc/cc-proxy/config.toml
systemctl start cc-proxy.service
```

Startup loaded five profiles/four providers, health check OK, and listener `0.0.0.0:11441`.

### 5.2 Production Chat regression

```text
DeepSeek mapped request: HTTP 200, end_turn
GLM mapped request: HTTP 200, end_turn
```

Journal records showed both requests using the legacy `OpenAI request built` path, not the Responses request path.

### 5.3 Production Responses text

```text
gpt-5.6 mapped request: HTTP 200, end_turn
```

Journal recorded `Responses request built` and `Responses response usage` with upstream HTTP 200. The subsequent effort probe also confirmed that the upstream accepted `reasoning.effort=max` and `reasoning.effort=xhigh`; the current requested production wire value is `max`.

### 5.4 Production tool continuation

Two independent new requests were made with `tool_choice` forcing the configured function. Each initial response returned a real Anthropic `tool_use` id and `stop_reason=tool_use`; each continuation used that id once in `tool_result`.

```text
chain 1: initial HTTP 200 tool_use/id present; continuation HTTP 200 end_turn
chain 2: initial HTTP 200 tool_use/id present; continuation HTTP 200 end_turn
```

No placeholder id or previously consumed id was used.

The production journal recorded Responses input items with:

```text
role:user, function_call, function_call_output
```

### 5.5 Production SSE

```text
HTTP 200
Content-Type: text/event-stream
required events present: true
observed: message_start, content_block_start, content_block_delta, content_block_stop, message_delta, message_stop
```

### 5.6 Production long-prefix cache

A fixed system prefix of `191520` characters was used with fixed tools and a fixed final user message. Only valid HTTP 200 responses with parseable usage were counted.

Warmup:

```text
request 1: input=31215, cache_creation=31212, cache_read=0
```

Subsequent valid samples:

```text
request 2: input=31215, cache_read=31212, cache_creation=0
request 3: input=31215, cache_read=31212, cache_creation=0
request 4: input=31215, cache_read=31212, cache_creation=0
request 5: input=31215, cache_read=31212, cache_creation=0
request 6: input=31215, cache_read=31212, cache_creation=0
request 7: input=31215, cache_read=31212, cache_creation=0
request 8: input=31215, cache_read=31212, cache_creation=0
request 9: input=31215, cache_read=31212, cache_creation=0
request 10: input=31215, cache_read=31212, cache_creation=0
```

Observed cache-read ratio:

```text
31212 / 31215 = 99.990389%
```

This exceeds the required 90% threshold. An earlier request in the same run after warmup had a cache creation response; it was recorded as cache creation, not misclassified as a miss/read result. All later samples used for the threshold were valid HTTP 200 cache-read samples.

### 5.7 Final live state

```text
cc-proxy.service: active/running
MainPID: 1774840
ExecMainStartTimestamp: 2026-08-06 10:59:14 CST
11441 health: HTTP 200
11441 listener: present
11449 listener: absent
production binary SHA-256 == release SHA-256: true
production config mapping: claude-sonnet-4-6 -> gpt-5.6-luna
production gpt wire_api: responses
```

No rollback was required for this second deployment. The previous rollback remains recorded in Sections 3-4. No credentials, authorization values, or raw request content are included.

## 6. Updated decision

For the second deployment attempt, all required production business gates passed. The production service is currently running the verified Responses artifact and configuration.
