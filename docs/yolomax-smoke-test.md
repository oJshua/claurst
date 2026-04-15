# Yolomax Connector — Manual Smoke Test

## Prerequisites

- A running Yolomax dev proxy with `YOLOMAX_BASE_URL` set
- Claurst built from this branch

## Steps

### 1. Connect

```sh
YOLOMAX_BASE_URL=<dev-proxy> ./claurst
/connect yolomax
```

Expected:
- TUI displays `user_code` and `verification_uri`
- Browser opens to verification URI
- After authorization, "Connected to Yolomax successfully!" with available models listed

### 2. Planning session

```
/plan refactor the auth module
```

Expected:
- Proxy logs show `x-claurst-activity: planning` on incoming requests
- Model produces a plan without executing

### 3. Coding session

```
Write a hello-world function in src/example.rs
```

Expected:
- Proxy logs show `x-claurst-activity: coding`
- File is created normally

### 4. Subagent delegation

Use a prompt that triggers the Agent tool:

```
Research what test frameworks are available, then create a test file.
```

Expected:
- Proxy logs show `x-claurst-activity: subagent` for the sub-agent's requests
- Main loop shows `x-claurst-activity: coding`

### 5. Summarize (compaction)

Have a long conversation until auto-compaction triggers (~90% context).

Expected:
- Proxy logs show `x-claurst-activity: summarize` for the compaction request
- Conversation continues normally after compaction

### 6. Verify headers on proxy logs

All requests should include:
- `Authorization: Bearer <token>`
- `x-claurst-client-version: <version>`
- `x-claurst-session-id: <session-id>`
- `x-claurst-activity: <coding|planning|subagent|summarize|title>`

### 7. Verify fallback indicator

Force a degraded response by configuring the proxy to return 503 with
`x-claurst-degraded: 1`. The request should still succeed (fallback model).

### 8. Verify quota error UX

Force a 402 from the proxy. Expected:
- Friendly "Out of quota" error message
- No crash or retry loop

### 9. Verify token refresh

Force a 401 with `invalid_token` from the proxy. Expected:
- Transparent refresh attempt
- If refresh succeeds: request retries and succeeds
- If refresh fails: "Run /connect yolomax" error message
