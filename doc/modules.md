# Modules: Reusable Config Components

## Motivation

Today, every `nexus.toml` must define links, channels, power profiles, and
resource constraints from scratch. Common configurations (LoRa 915 MHz, Wi-Fi
2.4 GHz, Cat-5e Ethernet, ESP32 resource profile, coin-cell battery, etc.) are
copy-pasted between projects. This leads to:

- **Duplication** -- the same LoRa link definition appears in every LoRa project.
- **Error-prone setup** -- users must know RF parameters, cable RLGC values, and
  MCU specs to write even a basic config.
- **No composability** -- there is no way to share or version partial configs.

The **module system** addresses this by letting users import pre-defined
configuration fragments and by shipping Nexus with a **standard library** of
common hardware and protocol definitions.

---

## Feature Scope

### 1. TOML `use` Directive

A new top-level `use` key allows importing module files. Modules are plain TOML
files that define links, channels, and/or node profiles (partial node
templates). They use the same schema as the corresponding top-level sections of
`nexus.toml`.

```toml
use = [
    "lora/sx1276_915mhz",        # built-in standard library module
    "wifi/esp32_2_4ghz",          # another built-in
    "./my_modules/custom_link",   # user-defined, relative to config file
]
```

**Resolution rules:**

1. Paths starting with `./` or `/` are resolved relative to the config file
   directory (or as absolute paths).
2. Bare names (no leading `./` or `/`) are resolved from the standard library
   directory shipped with Nexus (`<nexus_install>/modules/`). An environment
   variable `NEXUS_MODULE_PATH` can prepend additional search directories
   (colon-separated, searched left to right).
3. The `.toml` extension is appended automatically if not present.
4. Modules may themselves contain `use` directives (transitive imports).
   Circular imports are detected and rejected.

### 2. Module File Format

A module file is a subset of `nexus.toml`. It may contain any combination of:

```toml
# modules/lora/sx1276_915mhz.toml

[links.lora_915]
medium.type = "wireless"
medium.wavelength_meters = 0.328   # 915 MHz
medium.gain_dbi = 2.0
medium.rx_min_dbm = -137.0
medium.tx_min_dbm = -4.0
medium.tx_max_dbm = 20.0
medium.shape = "omni"
packet_loss = "1 / (1 + exp((rssi + 120) / 5))"

[links.lora_915.delays]
transmission.rate = 300
transmission.data = "bit"
transmission.time = "s"
propagation.rate = "d / 299792.458"
propagation.distance = "km"
propagation.time = "s"

[channels.lora]
link = "lora_915"
type = { type = "exclusive", ttl = 2000, unit = "ms", max_size = 255 }
```

**Allowed top-level keys in a module file:**

| Key | Description |
|-----|-------------|
| `use` | Transitive imports (other modules this one depends on) |
| `links` | Link definitions (same schema as `nexus.toml`) |
| `channels` | Channel definitions (same schema as `nexus.toml`) |
| `profiles` | Node profiles -- reusable partial node templates (new; see below) |

Module files **cannot** contain `params` or `nodes`. Those are simulation-
specific and belong only in the user's `nexus.toml`.

### 3. Node Profiles

A **profile** is a named, reusable fragment of node configuration. It can
define resources, power states, power sources/sinks, and channel energy costs.
Profiles live in module files under the `[profiles]` table.

```toml
# modules/boards/esp32_devkit.toml

[profiles.esp32]
[profiles.esp32.resources]
clock_rate = 240
clock_units = "mhz"
ram = 512
ram_units = "kb"

[profiles.esp32.power_states]
deep_sleep = { rate = 10,  unit = "uw", time = "s" }
light_sleep = { rate = 800, unit = "uw", time = "s" }
modem_sleep = { rate = 20,  unit = "mw", time = "s" }
active      = { rate = 100, unit = "mw", time = "s" }
wifi_active = { rate = 180, unit = "mw", time = "s" }

[profiles.esp32.power_sinks]
mcu = { rate = 30, unit = "mw", time = "s" }
```

Profiles are applied to nodes with a new `profile` key:

```toml
# User's nexus.toml
use = ["boards/esp32_devkit", "lora/sx1276_915mhz"]

[nodes.sensor]
profile = "esp32"
deployments = [
    { position = { point = [10, 0, 0], unit = "km" },
      charge = { max = 1000, quantity = 1000, unit = "mwh" },
      initial_state = "active" }
]

# User-specified fields override or merge with the profile:
[nodes.sensor.power_sinks]
gps = { rate = 25, unit = "mw", time = "s" }   # added on top of profile's "mcu"

[nodes.sensor.channel_energy.lora]
tx = { quantity = 120, unit = "uj" }

[[nodes.sensor.protocols]]
name = "firmware"
root = "./firmware"
build = "make"
runner = "./bin/main"
publishers = ["lora"]
subscribers = ["lora"]
```

### 4. Merge Semantics

When modules are imported and profiles are applied, values are merged with
well-defined rules:

**Between modules (via `use`):**

- **Links and channels:** Names must be globally unique. If two modules define
  the same link or channel name, the config is rejected with an error listing
  both sources. The user's `nexus.toml` definitions take precedence over any
  module -- if the user defines a link with the same name as a module's link,
  the user's definition wins (with a warning).
- **Profiles:** Same uniqueness rules as links/channels.
- **Import order:** Modules listed earlier in `use` are loaded first. Transitive
  dependencies are loaded depth-first. Each module file is loaded at most once
  (deduplication by resolved path).

**Between a profile and inline node config:**

- **Maps** (`power_states`, `power_sources`, `power_sinks`, `channel_energy`):
  The user's keys are merged on top of the profile's keys. If the same key
  exists in both, the user's value wins.
- **Scalars** (`resources`, etc.): The user's value replaces the profile's
  value entirely for each field that is specified. Unspecified fields fall
  through to the profile.
- **Absent fields:** If neither the profile nor the user specifies a field, the
  normal defaults apply.

### 5. Standard Library

Nexus ships with a curated set of modules organized by category:

```
modules/
  lora/
    sx1276_915mhz.toml        # Semtech SX1276 at 915 MHz (Americas)
    sx1276_868mhz.toml        # Semtech SX1276 at 868 MHz (EU)
    sx1262_915mhz.toml        # Semtech SX1262 at 915 MHz
    ra01_433mhz.toml          # AI-Thinker Ra-01 at 433 MHz
  wifi/
    wifi_2_4ghz.toml           # Generic 802.11b/g/n at 2.4 GHz
    wifi_5ghz.toml             # Generic 802.11ac at 5 GHz
    esp32_wifi.toml            # ESP32 Wi-Fi (includes board profile)
  wired/
    ethernet_cat5e.toml        # Cat-5e copper Ethernet
    ethernet_cat6.toml         # Cat-6 copper Ethernet
    usb_2_0.toml               # USB 2.0 full-speed
    serial_uart.toml           # Generic UART serial
  boards/
    esp32_devkit.toml          # ESP32 DevKit v1
    esp32_s3.toml              # ESP32-S3
    arduino_uno.toml           # ATmega328P @ 16 MHz
    arduino_mega.toml          # ATmega2560 @ 16 MHz
    stm32f4.toml               # STM32F4 Discovery
    rpi_pico.toml              # Raspberry Pi Pico (RP2040)
    rpi_zero_w.toml            # Raspberry Pi Zero W
  batteries/
    cr2032.toml                # CR2032 coin cell (~225 mAh @ 3V)
    18650.toml                 # 18650 Li-ion (~3000 mAh @ 3.7V)
    lipo_1s_500mah.toml        # 1S LiPo 500 mAh
  energy/
    solar_small.toml           # Small solar panel (~100 mW peak)
    solar_medium.toml          # Medium panel (~1W peak, day/night cycle)
    energy_harvester.toml      # Thermoelectric / vibration harvester
  scenarios/
    star_network.toml          # Common star topology helpers
    mesh_lora.toml             # LoRa mesh with typical settings
```

Each module file includes a header comment documenting the real-world component
it models, the source of its parameters, and any simplifying assumptions.

### 6. Listing and Inspecting Modules

New CLI subcommands for discoverability:

```
nexus modules list                    # List all available modules (stdlib + NEXUS_MODULE_PATH)
nexus modules list --category lora    # Filter by category/directory
nexus modules show lora/sx1276_915mhz # Print module contents with descriptions
nexus modules verify nexus.toml       # Check all `use` imports resolve and no conflicts exist
```

---

## Implementation Plan

### Phase 1: Module Loading Infrastructure

**Goal:** Parse `use` directives, resolve module paths, load and merge module
TOML files into the existing `parse::Simulation` structure.

**Files to modify:**

- `config/src/parse.rs` -- Add `use: Option<Vec<String>>` to `Simulation`. Add
  a `ModuleFile` struct (like `Simulation` but only `use`, `links`, `channels`,
  `profiles`; rejects `params`/`nodes`).
- `config/src/lib.rs` -- New `resolve_modules()` function:
  1. Collect the `use` list from the parsed TOML.
  2. Resolve each entry to a filesystem path (stdlib dir, `NEXUS_MODULE_PATH`,
     relative).
  3. Parse each module file as `ModuleFile`.
  4. Recursively resolve transitive `use` directives (track visited set for
     cycle detection).
  5. Merge all loaded links/channels/profiles into the main
     `parse::Simulation`, checking for conflicts.
  6. Call this before `ast::Simulation::validate()`.
- `config/src/validate.rs` -- Add conflict-detection logic for duplicate names
  across module boundaries. Emit source-file information in error messages.

**New files:**

- `config/src/module.rs` -- Module resolution, path search, cycle detection,
  and merge logic.

**Tests:**

- Module with one link import resolves correctly.
- Transitive `use` chains work.
- Circular `use` is rejected with clear error.
- Duplicate link/channel names across modules are rejected.
- User definitions override module definitions (with warning).
- `NEXUS_MODULE_PATH` is respected.
- Relative paths resolve from config file directory.
- Module files with `params` or `nodes` are rejected.

### Phase 2: Node Profiles

**Goal:** Implement the `profiles` table in modules and the `profile` key on
nodes.

**Files to modify:**

- `config/src/parse.rs` -- Add `profiles: Option<HashMap<String, NodeProfile>>`
  to `ModuleFile`. Add `profile: Option<String>` to `Node`. Define
  `NodeProfile` struct (resources, power_states, power_sources, power_sinks,
  channel_energy).
- `config/src/validate.rs` -- During node validation, if `profile` is set:
  1. Look up the profile from merged module data.
  2. Apply merge semantics (maps: union with user-wins; scalars: user-wins per
     field).
  3. Produce the final `ast::Node` as if the user had written it inline.
- `config/src/module.rs` -- Include profiles in merge/conflict logic.

**Tests:**

- Profile applies resources correctly.
- Profile power_states merge with user overrides.
- Unknown profile name is rejected.
- Profile with no user overrides works.
- Profile fields are fully overridable.

### Phase 3: Standard Library Modules

**Goal:** Create the `modules/` directory tree with curated, well-documented
module files.

**Steps:**

1. Create directory structure under `modules/` in the repo root.
2. Write each module file with:
   - Header comment citing data source (datasheet, spec, etc.).
   - Realistic parameters derived from component datasheets.
   - Sensible defaults for delay models and error expressions.
3. Add integration tests that parse each stdlib module.
4. Add a `build.rs` or install step that makes the stdlib path discoverable
   at runtime (e.g., embed the path via `env!()` or use a well-known relative
   path from the binary).

**Priority modules (first batch):**

1. `lora/sx1276_915mhz.toml` -- most relevant to existing case studies.
2. `boards/arduino_mega.toml` -- matches the Ring Routing case study.
3. `boards/esp32_devkit.toml` -- matches the LoRaMesher case study.
4. `batteries/cr2032.toml` -- simple, well-known battery.
5. `wired/serial_uart.toml` -- simple wired baseline.
6. `energy/solar_small.toml` -- demonstrates piecewise power source.

### Phase 4: CLI Subcommands

**Goal:** Add `nexus modules {list,show,verify}` commands.

**Files to modify:**

- `cli/src/main.rs` (or equivalent) -- Add `modules` subcommand with `list`,
  `show`, and `verify` sub-subcommands.

**Behavior:**

- `list`: Walk stdlib dir + `NEXUS_MODULE_PATH`, print module paths grouped
  by category. Optionally filter with `--category`.
- `show <module>`: Resolve the module, print its TOML with the header comment.
- `verify <config>`: Parse the config, resolve all modules, report any
  conflicts or missing references without running a simulation.

### Phase 5: Documentation and Migration

**Goal:** Update existing docs and examples to use modules where appropriate.

- Update `doc/config-reference.md` with `use` and `profile` documentation.
- Convert at least 2-3 existing examples to use modules (e.g., `arduino/`,
  `energy_framework/`) to demonstrate the before/after improvement.
- Add a `doc/modules.md` quick-start guide (this document, trimmed to
  user-facing content).

---

## Design Decisions and Rationale

### Why not TOML native includes?

TOML has no include mechanism. Alternatives like pre-processing with
envsubst/m4 or using a different format (JSON Schema `$ref`, Dhall, CUE) would
add tooling dependencies and break the simplicity of "one TOML file." The `use`
directive keeps everything in TOML while being handled at the application layer.

### Why separate profiles from nodes?

Nodes require protocols (user-specific code paths), deployments (positions),
and simulation-specific charge values. These cannot be meaningfully shared.
Profiles capture the *hardware characteristics* (resources, power) that are
inherent to a board/chip and reusable across projects. Keeping them separate
avoids the complexity of "node inheritance" and partial node merging.

### Why user-wins merge instead of error-on-conflict?

For the profile-to-node merge, user-wins is the natural semantic: "I'm using
an ESP32 profile but overriding the clock rate." For module-to-module merges
(two modules defining the same link name), we error because the user likely
didn't intend the collision and can't see both definitions. The user's own
`nexus.toml` overrides modules because it represents explicit intent.

### Why not parameterized modules?

A parameterized module system (e.g., `use = [{ module = "lora/sx1276", frequency = 915 }]`)
adds significant complexity (template variables, type checking, default
values). The simpler approach -- define concrete variants as separate modules
(`sx1276_915mhz.toml` vs `sx1276_868mhz.toml`) and let users override specific
fields inline -- covers the common cases with far less machinery. If
parameterization proves necessary later, it can be added as a backward-
compatible extension.

### Why not allow `params` or `nodes` in modules?

`params` are simulation-global (seed, timestep, root dir) and make no sense to
share. `nodes` require protocols with filesystem paths, making them inherently
project-specific. Restricting modules to links, channels, and profiles keeps
them genuinely portable.
