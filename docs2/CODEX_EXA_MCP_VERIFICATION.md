# Codex Exa MCP Verification

Issue: #738

## Source Of Truth

The runtime source of truth is the user-level Codex config at:

```text
C:\Users\hotra\.codex\config.toml
```

The expected Exa MCP stanza is:

```toml
[mcp_servers.exa]
url = "https://mcp.exa.ai/mcp?tools=web_search_exa,web_fetch_exa"
enabled_tools = ["web_search_exa", "web_fetch_exa"]
required = true
startup_timeout_sec = 30.0
tool_timeout_sec = 90.0
```

`required = true` is intentional. If Exa cannot initialize, Codex startup should fail loudly instead of silently dropping the research tools.

## Why This Exists

During #720/#738, `codex mcp list` showed Exa configured, but the already-running Codex session's deferred tool search did not expose `web_search_exa` or `web_fetch_exa`. Direct MCP probing proved the hosted Exa endpoint was healthy, and a fresh `codex exec` process successfully executed an `mcp_tool_call` against server `exa`.

Root cause: the long-lived session had a stale model-visible tool snapshot. New Codex sessions read the current config and can call Exa.

## Verification

Use fresh Codex processes for the decisive check; do not rely on an existing long-lived session after changing MCP config.

```powershell
codex mcp get exa
```

Expected:

- `enabled: true`
- `enabled_tools: web_search_exa, web_fetch_exa`
- `transport: streamable_http`
- URL includes `?tools=web_search_exa,web_fetch_exa`
- startup timeout is `30`
- tool timeout is `90`

Direct MCP endpoint readback should list exactly the two default tools:

- `web_search_exa`
- `web_fetch_exa`

Fresh Codex action probe:

```powershell
codex exec --cd 'C:\code\Calyx' --json --output-last-message 'C:\code\Calyx\tmp\codex-exa-call-probe-output.txt' 'Use the MCP tool named web_search_exa to search for "Exa MCP Codex". If and only if that exact tool is unavailable, reply exactly EXA_UNAVAILABLE. Do not use any other search tool.' | Set-Content -LiteralPath 'C:\code\Calyx\tmp\codex-exa-call-probe-events.jsonl'
Select-String -LiteralPath 'C:\code\Calyx\tmp\codex-exa-call-probe-events.jsonl' -Pattern '"type":"mcp_tool_call","server":"exa","tool":"web_search_exa"'
```

Expected: the event stream contains an actual MCP tool call to `server="exa"` and `tool="web_search_exa"`.

## Edge Checks

1. Required startup fail-closed:
   - Run a fresh Codex process with `-c 'mcp_servers.exa.url="http://127.0.0.1:9/mcp"'`.
   - Expected: startup/tool build fails instead of silently omitting Exa.

2. Tool allow-list:
   - Ask a fresh Codex process to call `web_search_advanced_exa`.
   - Expected: no successful Exa advanced-search call appears, because only `web_search_exa` and `web_fetch_exa` are enabled.

3. Fetch path:
   - Ask a fresh Codex process to call `web_fetch_exa` for `https://exa.ai/docs/reference/exa-mcp`.
   - Expected: the event stream contains an MCP tool call to `server="exa"` and `tool="web_fetch_exa"`.

## References

- Exa MCP docs: https://exa.ai/docs/reference/exa-mcp
- Exa MCP repository: https://github.com/exa-labs/exa-mcp-server
- Codex MCP docs: https://developers.openai.com/codex/mcp
- Codex config reference: https://developers.openai.com/codex/config-reference
