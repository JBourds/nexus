# CLI Reference

Nexus provides two binaries: `nexus` (CLI simulator) and a GUI application.
This document covers the CLI.

**Source files:** `runner/src/cli.rs` (argument definitions), `cli/src/main.rs`
(subcommand dispatch).

---

## Global Flags

These flags apply to `nexus simulate` and affect output formatting:

| Flag | Default | Description |
|------|---------|-------------|
| `--fmt <format>` | `csv` | Output format for protocol summaries |
| `--dest <dest>` | `stdout` | Output destination: `stdout` or `file` |
| `-n <count>` | 1 | Run multiple independent simulations |
| `--root <path>` | — | Override the FUSE mount point location |

---

## `nexus simulate <config>`

Run a new simulation from a TOML configuration file.

```bash
nexus simulate nexus.toml
nexus simulate nexus.toml --root /tmp/nexus_mount
nexus simulate nexus.toml -n 5 --dest file
```

**What happens:**
1. Parse and validate the TOML config (including module resolution)
2. Create a timestamped output directory under `params.root`
3. Serialize a config snapshot (with CRC32 checksum) for replay
4. Build all protocol code (if `build` commands are specified)
5. Spawn protocol processes with cgroup v2 resource controls
6. Mount the FUSE filesystem
7. Run the discrete-event kernel loop
8. Collect and output protocol summaries (exit codes, stdout/stderr paths)

---

## `nexus replay <logs_path>`

Replay a previously recorded simulation from its output directory. The replay
uses the serialized config snapshot and TX log to reproduce the exact message
sequence without running protocol processes.

```bash
nexus replay ~/simulations/2024-01-15T10-30-00/
```

The `logs_path` must contain the `nexus.toml` config snapshot written during
the original simulation.

---

## `nexus logs <logs_path>`

Print raw binary logs from a simulation output directory as human-readable
text or CSV.

```bash
nexus logs ~/simulations/2024-01-15T10-30-00/
```

---

## `nexus parse <trace_file>`

Parse and analyze a `.nxs` binary trace file. This is the primary tool for
post-simulation analysis.

```bash
# Print all events
nexus parse trace.nxs

# Print only TX and RX events
nexus parse trace.nxs --events tx,rx

# Filter by node and timestep range
nexus parse trace.nxs --nodes sensor,gateway --from 100 --to 500

# Output as JSON Lines
nexus parse trace.nxs --output jsonlines

# Print only the header (metadata)
nexus parse trace.nxs --header_only

# Decode payloads through an external command
nexus parse trace.nxs --adapter "python3 decode.py"
```

### Parse Flags

| Flag | Description |
|------|-------------|
| `--events <types>` | Comma-separated event types: `tx`, `rx`, `drop`, `position`, `energy`, `motion` |
| `--nodes <names>` | Comma-separated node names (supports base-name prefix matching for deployed nodes) |
| `--channels <names>` | Comma-separated channel names |
| `--from <timestep>` | Start timestep (inclusive) |
| `--to <timestep>` | End timestep (inclusive) |
| `--output <format>` | Output format: `text` (default), `json`, `jsonlines` |
| `--adapter <command>` | External command for payload decoding (receives payload on stdin) |
| `--header_only` | Print only the trace header summary, then exit |

### Event Types

| Type | Description |
|------|-------------|
| `tx` | Message sent by a protocol |
| `rx` | Message received by a protocol |
| `drop` | Message dropped (below sensitivity, packet loss, TTL expired, buffer full) |
| `position` | Node position update |
| `energy` | Battery charge snapshot |
| `motion` | Motion pattern change |

---

## `nexus modules list [--category <category>]`

List all available modules from the standard library and any directories in
the `NEXUS_MODULE_PATH` environment variable.

```bash
nexus modules list
nexus modules list --category lora
nexus modules list --category boards
```

The `--category` flag filters modules by directory prefix.

---

## `nexus modules show <module>`

Print the contents of a specific module file, resolved using the same search
logic as the simulator.

```bash
nexus modules show lora/sx1276_915mhz
nexus modules show boards/esp32_devkit
```

---

## `nexus modules verify <config>`

Parse a configuration file, resolve all module imports, apply profiles, and
run full validation. Reports either `OK` or a structured error. Useful for
CI checks and debugging import conflicts without launching a simulation.

```bash
nexus modules verify nexus.toml
```

---

## Environment Variables

| Variable | Description |
|----------|-------------|
| `NEXUS_MODULE_PATH` | Colon-separated list of directories to search for module files (in addition to the built-in standard library) |

---

## Building

Nexus uses a Cargo workspace. Build commands:

```bash
# Build everything (release mode)
cargo build --release

# Build only the CLI binary
cargo build --release -p cli

# Build only the GUI binary
cargo build --release -p gui

# Run tests
cargo test

# Using the justfile
just build    # cargo build --release
just cli      # build CLI with cgroup delegation
just gui      # build GUI
```

The simulation requires cgroup v2 delegation. When running manually (not via
`just`), wrap the command:

```bash
systemd-run --user --scope -p "Delegate=true" -- nexus simulate nexus.toml
```
