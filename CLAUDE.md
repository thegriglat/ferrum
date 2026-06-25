# CLAUDE.md — project guide for AI assistants

## Project name

**Ferrum** — a Pingora-based WAF. The final binary is `ferrum`, built by `xferrum`.

## Build & test

```bash
make build       # xferrum build → ./target/debug/ferrum
make test        # cargo test --workspace --exclude ferrum-e2e
make test-e2e    # build binary, then run HTTP-based e2e tests
make lint        # cargo clippy --workspace -- -D warnings
```

All commands must pass before a step is considered done. `cargo check --workspace` and `cargo clippy --workspace -- -D warnings` must both be clean (zero errors, zero warnings) before reporting a task complete.

Run `cargo fmt` after final code implementation and before checks.

## Repository layout

| Path                      | Role                                                                                                                                                                                                                                                                                                                                            |
| ------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ferrum-sdk/`             | Public SDK: `Plugin`, `Sensor`, `SensorEntry`, `SensorFactory`, `Transformer`, `TransformerEntry`, `TransformerFactory`, `Hook`, `HookEntry`, `HookFactory`, `Terminator`, `TerminatorFactory`, `HttpSession`, `Evaluator`, `Score`, `NodeKind`, `NodeEntry`, `PluginFactory`, `Clock`, `RequestContext`, `ProviderId`, `init_from_inventory()` |
| `ferrum-core/`            | Score engine (`engine.rs`), registry (`registry.rs`), TOML config loader (`config.rs`)                                                                                                                                                                                                                                                          |
| `ferrum-graph/`           | Config visualiser binary — reads `ferrum.toml`, emits Graphviz DOT to stdout                                                                                                                                                                                                                                                                    |
| `ferrum/`                 | Pingora proxy **library** (`proxy.rs`, `loader.rs`, `metrics.rs`). No binary. Exports `pub fn run(config_path: &str)`.                                                                                                                                                                                                                          |
| `xferrum/`                | CLI tool — scans `plugins/` and builds a ferrum binary with all plugins linked                                                                                                                                                                                                                                                                  |
| `ferrum-e2e/`             | E2E integration tests — spawns the real binary, tests over HTTP                                                                                                                                                                                                                                                                                 |
| `plugins/sensors/*/`      | Sensor plugins — depend only on `ferrum-sdk`                                                                                                                                                                                                                                                                                                    |
| `plugins/transformers/*/` | Transformer plugins — depend only on `ferrum-sdk`                                                                                                                                                                                                                                                                                               |
| `plugins/terminators/*/`  | Terminator plugins — depend only on `ferrum-sdk`                                                                                                                                                                                                                                                                                                |
| `plugins/hooks/*/`        | Hook plugins — depend only on `ferrum-sdk`                                                                                                                                                                                                                                                                                                      |

## xferrum — build tool

`xferrum` replaces the old `ferrum` binary. It scans `plugins/` and generates a temporary Cargo workspace that links all plugins, then compiles it.

```bash
# Default: auto-scan ./plugins/ in current directory
xferrum build --output ./ferrum

# With external plugins
xferrum build \
  --plugin github.com/user/my-sensor \
  --plugin github.com/user/my-sensor@v1.2.3 \
  --plugin ./local/path/to/plugin \
  --plugin my-crate@1.0 \
  --output ./ferrum
```

Plugin spec formats:

- `github.com/owner/repo` → `{ git = "https://github.com/owner/repo" }`
- `github.com/owner/repo@v1.2.3` → `{ git = "...", tag = "v1.2.3" }`
- `./path` or `/abs/path` → `{ path = "..." }`
- `crate@1.0` → `{ version = "1.0" }` (crates.io)

## Plugin registration

All plugins self-register via `inventory::submit!`. No explicit `init()` calls or plugin lists in the runner.

```rust
// In any plugin crate (ferrum-sdk re-exports inventory):
ferrum_sdk::inventory::submit! {
    ferrum_sdk::SensorFactory {
        name: "my-sensor",
        build: |args, own_id| MySensor.into_entry(args, own_id),
    }
}
```

At runtime, `ferrum_sdk::init_from_inventory()` (called inside `ferrum::run()`) iterates all submitted factories and registers them. DCE is prevented by `use my_plugin as _` in the generated `main.rs` — the symbol reference forces the linker to include plugin object files.

## Plugin node types

All four plugin types implement the `Plugin: Send + Sync` marker trait. They correspond to the `NodeKind` enum variants: `Sensor`, `Transformer`, `Hook`, `Terminator`.

### Sensors

1. Implement `Sensor` — declare your own `Args` struct, no `Any`.
2. `evaluate(&self, ctx: &mut RequestContext, args: &Self::Args) -> Score` — called per-request.
3. Declare dependencies via `dep_ids` — always empty for sensors.
4. Use `into_entry(self, args, own_id)` or build `SensorEntry` manually if you need `own_id` during compile.
5. Register with `ferrum_sdk::inventory::submit! { SensorFactory { name, build } }`.

### Transformers

1. Implement `Transformer` — declare your own `Args` struct.
2. `evaluate(&self, args: &Self::Args, eval: &dyn Evaluator) -> Score` — no access to `RequestContext`.
3. Declare all dependencies via `dep_ids(args)` — the engine pre-populates scores before calling `evaluate`.
4. Use `into_entry(self, args)` for the build closure.
5. Register with `ferrum_sdk::inventory::submit! { TransformerFactory { name, build } }`.

### Hooks

1. Implement `Hook` — declare your own `Args` struct.
2. `trigger(&self, ctx: &RequestContext, args: &Self::Args)` — read-only; must be non-blocking (send to an async channel, do not await).
3. Use `into_entry(self, args)` for the build closure.
4. Register with `ferrum_sdk::inventory::submit! { HookFactory { name, build } }`.
5. Hooks appear in `[hooks.*]` config sections with an optional `next` action (rule, hook, or terminator).

### Terminators

1. Implement `Terminator` — `async fn execute(&self, ctx: &RequestContext, session: &mut dyn HttpSession) -> bool`. Return `true` if the request was handled, `false` to pass.
2. Use `session.respond(status, headers, body)` to write an HTTP response, or `session.close()` to drop the connection.
3. Register with `ferrum_sdk::inventory::submit! { TerminatorFactory { name, build } }`.
4. Built-in plugins: `"block"` (`plugins/terminators/block`), `"pass"` / `"bypass"` (`plugins/terminators/pass`), `"drop"` (`plugins/terminators/drop`), `"redirect"` (`plugins/terminators/redirect`), `"rate_limited"` (`plugins/terminators/rate-limited`), `"timeout"` (`plugins/terminators/timeout`).

### Common rules

- Only `ferrum-sdk` may be imported from the workspace; no cross-plugin deps.
- All `compile_args` calls are at startup — panics here are acceptable for misconfiguration.
- `evaluate` / `trigger` is called per-request and must be `&self` (stateless reads only).
- Every plugin type must `impl Plugin for MyType {}` (required by the supertrait bound).

## Scoring semantics

- Scores are `Score(u8)` in `[0, 100]`.
- `Score::from(0.75f32)` → `Score(75)` (clamped, rounded).
- `Score::from(true)` → `Score(100)`, `Score::from(false)` → `Score(0)`.
- `Score::and(a, b)` → `min(a, b)`, `Score::or(a, b)` → `max(a, b)`, `!score` → `100 - score`.
- Each provider result is cached in `RequestContext.cache` (keyed by `ProviderId`) for the lifetime of a request.
- Rule `threshold` is a `Score` in `[0, 100]` in config (deserialized directly from the TOML integer); a rule fires when `score >= threshold`.

## Config format

TOML with six sections: `[server]`, `[sensors.*]`, `[transformers.*]`, `[hooks.*]`, `[terminators.*]`, `[[rules]]`. An optional top-level `entry` field names the DAG entry point.

Provider names are hashed with `FxHasher` to produce `ProviderId` at startup. All runtime lookups use `ProviderId` — no string searches on the hot path. Duplicate names → panic at startup. Unknown `input`/action references → panic at startup.

### Entry point

By default the entry point is the first `[[rules]]`. Use `entry` to override:

```toml
entry = "audit"     # can be a hook name, rule id, or terminator name
```

This allows hooks to fire before any rule, or a bare terminator to block unconditionally.

### Rule format

```toml
[[rules]]
id = "check-sqli"          # unique id
input = "sqli_combined"    # name of a sensor or transformer
threshold = 50             # fire when score >= this (integer 0–100)
if_action = "block_403"    # rule id, hook name, or terminator name (score >= threshold)
else_action = "check-geo"  # rule id, hook name, or terminator name (score < threshold)
buffer_body = true         # buffer the request body before evaluation (optional)
```

Actions resolve in order: rule id → hook name → named terminator → inline terminator plugin → panic. Omitting an action is equivalent to `pass`.

### Hook format

```toml
[hooks.audit]
plugin = "hook-audit-log"  # registered hook plugin name
next = "check-ip"          # action to continue after the hook fires (optional → pass)
<plugin args>
```

### Logic operators (transformer plugins)

| Plugin           | TOML args                              | Score                                      |
| ---------------- | -------------------------------------- | ------------------------------------------ |
| `"or"`           | `inputs = ["a", "b"]`                  | `max(inputs)`                              |
| `"and"`          | `inputs = ["a", "b"]`                  | `min(inputs)`                              |
| `"not"`          | `input = "a"`                          | `100 - score`                              |
| `"weighted_sum"` | `[inputs] table: name = weight`        | `Σ(weight × score)`, clamped to 100        |
| `"clamp"`        | `input = "a"`, `low = 20`, `high = 80` | linear remap of `[low, high]` → `[0, 100]` |

## Registry and NodeEntry

`PluginRegistry` (in `ferrum-core`) holds all four node types keyed by `ProviderId`:

- `register_node(id, NodeEntry)` — unified registration; `NodeEntry` is `Sensor | Transformer | Hook | Terminator`.
- `get_terminator(id) -> Option<Arc<dyn Terminator>>` — used by the proxy action loop.
- `evaluate_sensor`, `evaluate_transformer`, `trigger_hook` — called by the engine.

`PluginFactory::kind() -> NodeKind` maps any registered factory to its node type.

## Code style

- No comments unless the WHY is non-obvious.
- All doc comments (`///`) in English on every public item.
- No ML, no hot reload, no ModSecurity.
- `thiserror` for error types; `tracing` for logging; `prometheus` for metrics.
- Prefer `cargo add <crate>` (latest version) over manually pinning versions.

## Observability

- Audit log: JSON via `tracing` on stdout. Every block/pass decision is logged.
- Metrics: Prometheus on `:9090`. See `ferrum/src/metrics.rs`.
- Body buffer limit: 256 KiB. Oversized bodies disable body sensors for that request.

## Keeping docs current

After any structural change (new plugin type, new SDK export, new crate, scoring semantics change, config format change), update **both** `CLAUDE.md` and `README.md` to reflect the new state before closing the task.
