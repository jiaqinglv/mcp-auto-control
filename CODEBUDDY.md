# CODEBUDDY.md

This file provides guidance to CodeBuddy Code when working with code in this repository.

## What this is

`mcp-auto-control` is a Rust MCP (Model Context Protocol) server that drives two ESP32-S3 USB HID
devices — an auto mouse and an auto keyboard — by publishing binary frames to an MQTT broker.
The MCP client (CodeBuddy Code, MCP Inspector, …) sees mouse/keyboard tools; this server owns all
the translation into device frames.

Built on `rmcp` v3.4 (`rmcp::...`, `#[tool]`, `#[tool_router]`, …) with a `rumqttc` MQTT publisher.
There is **no perception channel**: the server cannot see the screen, and the devices never publish
anything back (no ACK, no status topic), so "success" only means "frame handed to the broker".

The device-side contract and the reasoning behind every constraint are documented in
`doc/方案设计.md` (Chinese); §3 there is the wire protocol, §10 the risk register.

## Commands

- Build: `cargo build` / `cargo build --release`
- Run: `cargo run -- --broker 192.168.3.15`
- Fast check: `cargo check --all-targets`
- Test all: `cargo test`
- Single test: `cargo test split_is_exact_for_negative_and_positive`
- One module: `cargo test device::keyboard`
- Integration test (tool registry): `cargo test --test tool_registry`
- Lint: `cargo clippy --all-targets`
- Format: `cargo fmt`
- Interactive: `npx @modelcontextprotocol/inspector cargo run`

`cargo run` alone looks like it hangs — it is a stdio server blocking on JSON-RPC, and it also
blocks until the broker is reached. Drive it with the Inspector or another MCP client. All tracing
goes to **stderr** on purpose: stdout is the MCP protocol channel and any stray write breaks it.

There is no automated test against a real broker or device. Two git-ignored scripts under `target/`
(which is why they are not checked in) cover end-to-end verification without hardware:

- `target/mqtt_smoke.py` — starts a fake broker, drives the real binary, asserts the MCP handshake
  and the exact bytes on the wire (13/10-byte lengths, frame splitting, Shift pairing, QoS/retain,
  monotonic `msg_id`). Assertions are deliberately order-agnostic.
- `target/order_probe.py` — the measurement behind the concurrency note below (prints the observed
  execution order of batched calls).

## Architecture

```
config.rs        CLI/env → Config (broker, topics, frame pacing, frame budget)
                      │
main.rs          tracing(stderr) → Publisher + spawned event loop → Devices → AutoControl over stdio
                      │
mqtt/codec.rs    pure encoders: 13-byte mouse frame, 10-byte key frame
mqtt/client.rs   Publisher: connection state + one publish per frame + per-frame pacing sleep
                      │
device/mod.rs    Devices + Sequence: virtual cursor, pressed-key/button state, action-sequence lock
device/mouse.rs  split_delta / frames_needed, click / scroll / drag semantics
device/keyboard.rs  firmware-derived key whitelist, ASCII→(keycode, shift), combo / text
                      │
mcp/mod.rs       AutoControl: merges the three tool routers, ServerHandler, shared helpers
mcp/tools/*.rs   thin tool wrappers (validate → call Devices → format result)
```

### The routing pattern you must not break

Tools are **split across three `impl` blocks**, one per domain, each annotated
`#[tool_router(router = mouse_tools, vis = "pub(crate)")]` (`src/mcp/tools/*.rs`). A single
`#[tool_router]` block cannot span modules, and the generated `*_tools()` accessors are private by
default — hence the explicit `vis`. They are combined in `AutoControl::all_tools()`
(`src/mcp/mod.rs`) with `ToolRouter`'s `+`, and `#[tool_handler(router = Self::all_tools())]`
feeds the result to the `ServerHandler` impl.

Adding a tool means: write the `#[tool]` fn in the right domain module, extend `EXPECTED_TOOLS` in
`tests/tool_registry.rs`, and add it to the `with_instructions(...)` text in `src/mcp/mod.rs` —
nothing derives that prose, and it is what the LLM client reads.

### Layer boundaries

- `mcp/tools/*` must stay thin: validate, call `devices()`, format. Never build raw frames there.
- **Only `devices` may touch `Publisher`.** `keyboard_release_all` / `emergency_stop` need a
  centrally tracked "what have we pressed" set, which only exists if every frame goes through
  `Sequence` (`src/device/mod.rs`) — it publishes one frame *and* updates cursor/buttons/keys.
- Every action acquires the `Sequence` lock, so mouse and keyboard frames never interleave and the
  frame interval actually throttles. Long drags hold it for their whole duration by design.

### Concurrency: the lock does not order *actions*

rmcp dispatches requests concurrently, so **the execution order of parallel tool calls is
undetermined** — measured with `target/order_probe.py`, four batched `mouse_move` calls ran in four
different orders across runs. The `Sequence` lock guarantees only that a single action is never
torn apart (a click can't interleave with a key press). Anything order-dependent must be triggered
by the client serially. This is documented in `INSTRUCTIONS` (`src/mcp/mod.rs`) and as R14 in
`doc/方案设计.md`; do not "fix" it by reordering publishes, it is a dispatch-layer property.

### Firmware invariants (breaking these breaks hardware)

- **Mouse frames must be exactly 13 bytes.** The firmware validates `len >= 10` but reads
  `payload[12]`; 10–12 bytes panics it, which stops feeding its watchdog and resets the chip.
- **Key codes must be in the firmware's `KEY_MAPPING`.** Anything else is silently ignored by the
  firmware, so the server keeps its own whitelist in `device/keyboard.rs` and **errors instead**.
  Values there were extracted from the exact rmk revision the keyboard firmware pins — do not
  "correct" them from the HID spec without re-checking.
- **Mouse coordinates are i8 relative deltas.** Absolute movement is emulated with a virtual cursor
  and therefore approximate; `split_delta` must always sum back to exactly the requested delta.
- `max_step` is capped at 127 in `Config::validate()`.
- Text input is ASCII-only by construction (no Unicode path exists on the device).

## Layout notes

- The `Counter` demo is gone: `src/common/` (rmcp's counter example) and the repo-root `common/`
  (its never-compiled historical copy) were both deleted, along with `src/keymap.rs`. Do not
  reintroduce them; the tool-router assembly they demonstrated is documented in
  `doc/方案设计.md` §5.3 and implemented in `src/mcp/mod.rs`.
- `Cargo.toml` deliberately enables only the `rmcp` features actually used
  (`server`, `macros`, `transport-io`, `schemars`). `axum` is no longer a direct dependency.
- `.cargo/config.toml` pins the resolver to the repo-local `Cargo.lock`.
- Comments and all user-facing strings (tool descriptions, errors, `with_instructions`) are in
  Chinese — keep new ones consistent.
