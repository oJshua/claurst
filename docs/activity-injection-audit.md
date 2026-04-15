# Activity-Tag Injection Points — Audit

Audit of the 5 call sites where a `RequestActivity` tag will be injected
before handing a request to the LLM provider. Each entry lists the file,
line, dispatch function, and what the call site does.

## 1. Coding — Main query loop

- **File:** `src-rust/crates/query/src/lib.rs`
- **Line:** 1087
- **Call:** `provider.create_message_stream(provider_request)`
- **Context:** Inside `run_query_loop()`. The `ProviderRequest` is built at
  lines 1061–1083. This is the primary agentic conversation loop — every
  user-facing turn dispatches here. When plan mode is **not** active
  (`config.agent_name != Some("plan")`), tag as `Coding`.

## 2. Planning — Plan-mode turns

- **File:** `src-rust/crates/query/src/lib.rs`
- **Line:** 1087 (same dispatch as Coding)
- **Call:** `provider.create_message_stream(provider_request)`
- **Context:** Same `run_query_loop()` dispatch. Plan mode is activated via
  the `EnterPlanModeTool` (or `/plan` command), which sets
  `config.agent_name = Some("plan")` and `permission_mode = Plan`. The
  activity tag should be set conditionally at `ProviderRequest` construction
  (lines 1061–1083): if `config.agent_name.as_deref() == Some("plan")`, tag
  as `Planning`; otherwise `Coding`.
- **Detection:** `config.agent_name` field on `QueryConfig` (line 119) or
  `tool_ctx.permission_mode == PermissionMode::Plan`.

## 3. Subagent — agent_tool dispatch

- **File:** `src-rust/crates/query/src/agent_tool.rs`
- **Line:** 465
- **Call:** `run_query_loop(client.as_ref(), &mut messages, ...)`
- **Context:** Inside `AgentTool::execute()`, synchronous mode. The
  `QueryConfig` is constructed at lines 348–370 with
  `agent_name: None`. The subagent's query loop will dispatch at
  `lib.rs:1087`, but the activity tag should be overridden to `Subagent` for
  all turns inside this sub-loop. Set `activity = Subagent` on the
  `QueryConfig` (or propagate it so the inner `ProviderRequest` carries it).
- **Background variant:** The same `QueryConfig` is cloned for background
  agents at line 389 (`ctx_bg = ctx.clone()`), which calls
  `run_query_loop` inside a `tokio::spawn` at approximately line 400. Both
  paths need the `Subagent` tag.

## 4. Summarize — Compaction and context-collapse

Two dispatch points, both in `compact.rs`:

### 4a. `summarise_head()` — regular compaction

- **File:** `src-rust/crates/query/src/compact.rs`
- **Line:** 616
- **Call:** `client.create_message_stream(request, handler)`
- **Context:** Called by `compact_conversation()` (line 291) and
  `reactive_compact()` (line 915). The `CreateMessageRequest` is built at
  lines 604–612. Tag as `Summarize`.

### 4b. `context_collapse()` — emergency compaction

- **File:** `src-rust/crates/query/src/compact.rs`
- **Line:** 1025
- **Call:** `client.create_message_stream(request, handler)`
- **Context:** Called when token usage hits ≥97% of context window
  (`should_context_collapse()`, line 775). The `CreateMessageRequest` is
  built at lines 1015–1022. Tag as `Summarize`.

### Callers in the query loop

Both compaction paths are invoked from `run_query_loop()` in `lib.rs`:
- `context_collapse` at line 1509
- `reactive_compact` at line 1533

## 5. Title — Session name generation

- **File:** `src-rust/crates/commands/src/lib.rs`
- **Line:** 4299
- **Call:** `provider.create_message(request)`
- **Context:** Inside the `/rename` command's `execute()` method, when no
  explicit name argument is provided. The `ProviderRequest` is built at
  lines 4282–4297. Tag as `Title`.

---

## Summary table

| Activity   | File                              | Line(s)     | Function / Method               |
|------------|-----------------------------------|-------------|---------------------------------|
| Coding     | `crates/query/src/lib.rs`         | 1087        | `run_query_loop()`              |
| Planning   | `crates/query/src/lib.rs`         | 1087        | `run_query_loop()` (plan mode)  |
| Subagent   | `crates/query/src/agent_tool.rs`  | 465         | `AgentTool::execute()`          |
| Summarize  | `crates/query/src/compact.rs`     | 616, 1025   | `summarise_head()`, `context_collapse()` |
| Title      | `crates/commands/src/lib.rs`      | 4299        | `RenameCommand::execute()`      |

## Key types to modify (Tasks 2–3)

- **`ProviderRequest`** (`crates/api/src/provider_types.rs:52`): add
  `activity: Option<RequestActivity>` field.
- **`QueryConfig`** (`crates/query/src/lib.rs:86`): could carry a default
  activity so `run_query_loop` automatically stamps it.
- **`CreateMessageRequest`** (used by compact.rs): the compaction paths use
  the raw Anthropic client, not the `LlmProvider` trait. The activity
  header will need to be injected at the HTTP layer (Task 3 `extra_headers`).

## `resolve_anthropic_api_base()` call sites (reference)

| File                              | Line(s)      | Purpose                              |
|-----------------------------------|-------------|---------------------------------------|
| `crates/core/src/lib.rs`         | 1352, 1360  | Definition + fallback resolution      |
| `crates/query/src/agent_tool.rs` | 227, 241, 562, 581 | Subagent client construction   |
| `crates/commands/src/lib.rs`     | 148, 1952   | Command client construction           |
| `crates/cli/src/main.rs`         | 564, 920    | CLI entry-point client construction   |
