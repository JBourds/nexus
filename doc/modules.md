# Modules: Reusable Config Components

## Table of Contents

1. [Motivation](#motivation)
2. [Quick Start](#quick-start)
3. [Feature Overview](#feature-overview)
   - [The `use` Directive](#1-the-use-directive)
   - [Module File Format](#2-module-file-format)
   - [Node Profiles](#3-node-profiles)
   - [Multi-Profile Layering](#4-multi-profile-layering)
   - [Merge Semantics](#5-merge-semantics)
   - [Standard Library](#6-standard-library)
   - [CLI Subcommands](#7-cli-subcommands)
4. [GUI Integration](#gui-integration)
5. [Design Decisions](#design-decisions)

---

## Motivation

Without modules, every `nexus.toml` must define links, channels, power profiles,
and resource constraints from scratch. Common configurations (LoRa 915 MHz, Wi-Fi
2.4 GHz, Cat-5e Ethernet, ESP32 resource profile, coin-cell battery, and so on)
are copy-pasted between projects. This leads to:

- **Duplication** -- the same LoRa link definition appears in every LoRa project.
- **Error-prone setup** -- users must know RF parameters, cable RLGC values, and
  MCU specs to write even a basic config.
- **No composability** -- there is no way to share or version partial configs.

The **module system** addresses this by letting users import pre-defined
configuration fragments via a `use` directive, and by shipping Nexus with a
**standard library** of common hardware and protocol definitions.

---

## Quick Start

The following config simulates a three-node LoRa mesh without specifying any
RF parameters by hand:

```toml
# nexus.toml
use = ["lora/sx1276_915mhz"]

[params]
timestep.length = 10
timestep.unit   = "ms"
timestep.count  = 10000
seed            = 7
root            = "~/simulations"

[nodes.sensor]
deployments = [{ position = { point = [0, 0, 0], unit = "km" } }]

[[nodes.sensor.protocols]]
name       = "fw"
runner     = "python3"
runner_args = ["sensor.py"]
publishers = ["lora"]

[nodes.relay]
deployments = [{ position = { point = [5, 0, 0], unit = "km" } }]

[[nodes.relay.protocols]]
name        = "fw"
runner      = "python3"
runner_args = ["relay.py"]
publishers  = ["lora"]
subscribers = ["lora"]

[nodes.gateway]
deployments = [{ position = { point = [10, 0, 0], unit = "km" } }]

[[nodes.gateway.protocols]]
name        = "fw"
runner      = "python3"
runner_args = ["gateway.py"]
subscribers = ["lora"]
```

The `use = ["lora/sx1276_915mhz"]` line imports the stdlib module, which
contributes a `lora_915` link definition (wireless medium, propagation and
transmission delays modeled from the SX1276 datasheet) and a `lora` channel
(exclusive, 255-byte max, 2-second TTL). The nodes reference `"lora"` as if it
had been written inline.

For more examples, see `examples/modules_solar_sensor/`, `examples/modules_lora_mesh/`,
and `examples/modules_mixed_network/`.

---

## Feature Overview

### 1. The `use` Directive

A top-level `use` key accepts a list of module specifiers. Modules are plain TOML
files that contribute links, channels, and node profiles to the simulation.

```toml
use = [
    "lora/sx1276_915mhz",        # standard library module
    "wifi/wifi_2_4ghz",           # another stdlib module
    "./my_modules/custom_link",   # user-defined, relative to nexus.toml
]
```

**Resolution rules (applied in order for each specifier):**

1. Specifiers starting with `./`, `../`, or `/` are resolved as filesystem paths
   relative to the config file's directory (or as absolute paths). No search is
   performed.
2. Bare names are searched left-to-right through each directory listed in the
   `NEXUS_MODULE_PATH` environment variable (colon-separated).
3. Bare names not found in `NEXUS_MODULE_PATH` are resolved against the standard
   library directory embedded in the binary at compile time.
4. The `.toml` extension is appended automatically when not already present.
5. Modules may themselves contain `use` directives. Transitive imports are
   resolved depth-first, relative to the importing module's directory.
6. Each module file is loaded at most once regardless of how many times it
   appears (deduplication by canonical path). Circular imports are detected and
   rejected with an error.

### 2. Module File Format

A module file is a restricted subset of `nexus.toml`. It may contain:

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
| `use` | Transitive imports -- other modules this one depends on |
| `links` | Link definitions (same schema as `nexus.toml`) |
| `channels` | Channel definitions (same schema as `nexus.toml`) |
| `profiles` | Node profiles -- reusable partial node templates (see below) |

Module files **cannot** contain `params` or `nodes`. Those keys are
simulation-specific and are rejected by the parser with a clear error.

Each stdlib module file includes a header comment documenting the real-world
component it models, the source of its parameters (datasheet, spec, etc.), and
any simplifying assumptions.

### 3. Node Profiles

A **profile** is a named, reusable fragment of node configuration. Profiles
capture the hardware characteristics of a board or chip -- resources and power
behavior -- that are stable across projects. They live in module files under the
`[profiles]` table.

```toml
# modules/boards/esp32_devkit.toml

[profiles.esp32]

[profiles.esp32.resources]
clock_rate  = 240
clock_units = "mhz"
ram         = 520
ram_units   = "kb"

[profiles.esp32.power_states]
deep_sleep  = { rate = 33,   unit = "uw", time = "s" }
light_sleep = { rate = 2640, unit = "uw", time = "s" }
modem_sleep = { rate = 66,   unit = "mw", time = "s" }
active      = { rate = 330,  unit = "mw", time = "s" }
wifi_active = { rate = 600,  unit = "mw", time = "s" }

[profiles.esp32.power_sinks]
mcu = { rate = 99, unit = "mw", time = "s" }
```

A node references a profile with the `profile` key, which accepts either a
single string or a list of strings:

```toml
use = ["boards/esp32_devkit", "lora/sx1276_915mhz"]

[nodes.sensor]
profile = "esp32"          # single profile

[nodes.gateway]
profile = ["esp32", "solar_small"]   # multiple profiles, applied in order
```

`NodeProfile` fields:

| Field | Type | Description |
|-------|------|-------------|
| `resources` | object | CPU clock rate, core count, RAM |
| `power_states` | map | Named active power consumption states |
| `power_sources` | map | Passive energy inputs (e.g. solar panel) |
| `power_sinks` | map | Passive energy draws (e.g. MCU baseline) |
| `channel_energy` | map | Per-channel TX/RX one-time energy costs |

### 4. Multi-Profile Layering

When a node specifies multiple profiles (`profile = ["board", "energy"]`), they
are applied in list order using the same first-wins merge logic as the
profile-to-node merge:

- **Resources (scalars):** The first profile in the list that specifies a given
  field sets that field. Later profiles only fill fields that are still unset.
  Any value the user writes inline beats all profiles.
- **Maps (`power_states`, `power_sources`, `power_sinks`, `channel_energy`):**
  Keys are accumulated across all profiles. When two profiles define the same
  key, the one that appears **first** in the `profile` list wins. The user's
  inline definitions always win over any profile.

This makes profile order meaningful: put the most specific or most important
profile first. A typical pattern is `profile = ["board", "energy"]`, where the
board profile establishes CPU and power-state definitions and the energy profile
adds a power source.

```toml
# Energy profile (energy/solar_small.toml) adds a piecewise-linear solar source.
# Board profile (boards/esp32_devkit.toml) contributes resources, power states, MCU sink.
# Together they produce a node with all of the above, with board values taking
# precedence for any overlapping keys.
[nodes.sensor]
profile = ["esp32", "solar_small"]
```

### 5. Merge Semantics

**Between modules (via `use`):**

- **Links and channels:** Names must be globally unique across all imported
  modules. If two modules define the same link or channel name, the config is
  rejected with an error naming both source files.
- **User overrides:** If `nexus.toml` defines a link or channel with the same
  name as an imported module's definition, the user's definition wins and a
  warning is emitted. This is intentional -- the user's file represents explicit
  override intent.
- **Profiles:** Same uniqueness rules as links and channels. Two imported modules
  may not both define a profile with the same name; the user's `nexus.toml` may
  override a module profile with a warning.
- **Import order:** Modules listed earlier in `use` are loaded first. Transitive
  dependencies are resolved depth-first. Each file is loaded at most once.

**Between a profile and inline node config:**

- **Maps** (`power_states`, `power_sources`, `power_sinks`, `channel_energy`):
  Union of all keys. For any key that exists in both the profile and the inline
  node definition, the user's value wins.
- **Resources (scalars):** Per-field fallback. Each resource field (`clock_rate`,
  `clock_units`, `ram`, `ram_units`, `cores`) is taken from the user's inline
  definition if present; otherwise it falls back to the profile's value.
- **Absent fields:** If neither the profile nor the user specifies a field, the
  normal simulation defaults apply.

### 6. Standard Library

Nexus ships with 24 module files organized into six categories. The stdlib
directory is embedded at compile time (via `config/build.rs`) and is always
searched as the final fallback for bare-name module specifiers.

```
modules/
  batteries/
    18650.toml              # 18650 Li-ion (~3000 mAh @ 3.7 V)
    cr2032.toml             # CR2032 coin cell (~225 mAh @ 3 V)
    lipo_1s_500mah.toml     # 1S LiPo 500 mAh
  boards/
    arduino_mega.toml       # ATmega2560 @ 16 MHz
    arduino_uno.toml        # ATmega328P @ 16 MHz
    esp32_devkit.toml       # ESP32 DevKit v1 (dual-core LX6 @ 240 MHz)
    esp32_s3.toml           # ESP32-S3
    rpi_pico.toml           # Raspberry Pi Pico (RP2040)
    rpi_zero_w.toml         # Raspberry Pi Zero W
    stm32f4.toml            # STM32F4 Discovery
  energy/
    energy_harvester.toml   # Thermoelectric / vibration harvester
    solar_medium.toml       # Medium panel (~1 W peak, day/night cycle)
    solar_small.toml        # Small panel (~100 mW peak, day/night cycle)
  lora/
    ra01_433mhz.toml        # AI-Thinker Ra-01 at 433 MHz
    sx1262_915mhz.toml      # Semtech SX1262 at 915 MHz
    sx1276_868mhz.toml      # Semtech SX1276 at 868 MHz (EU)
    sx1276_915mhz.toml      # Semtech SX1276 at 915 MHz (Americas)
  wifi/
    esp32_wifi.toml         # ESP32 Wi-Fi (includes board profile)
    wifi_2_4ghz.toml        # Generic 802.11b/g/n at 2.4 GHz
    wifi_5ghz.toml          # Generic 802.11ac at 5 GHz
  wired/
    ethernet_cat5e.toml     # Cat-5e copper Ethernet
    ethernet_cat6.toml      # Cat-6 copper Ethernet
    serial_uart.toml        # Generic UART serial
    usb_2_0.toml            # USB 2.0 full-speed
```

The `batteries/` and `energy/` modules define only profiles (no links or
channels). The `lora/`, `wifi/`, and `wired/` modules define links and channels,
and some also define board-specific profiles. The `boards/` modules define only
profiles.

### 7. CLI Subcommands

Three subcommands support module discovery and validation:

```
nexus modules list                      # List all available modules (stdlib + NEXUS_MODULE_PATH)
nexus modules list --category lora      # Filter by category/directory
nexus modules show lora/sx1276_915mhz   # Print module file contents
nexus modules verify nexus.toml         # Parse config, resolve all imports, report conflicts
```

**`list`** walks the standard library directory and any directories in
`NEXUS_MODULE_PATH`, printing module specifiers grouped by root directory.
Results are sorted by filename within each directory.

**`show`** resolves the specifier using the same path-search logic as the
simulator and prints the raw TOML content, including its header comment.

**`verify`** runs the full config parse pipeline -- TOML parsing, module
resolution, profile application, and AST validation -- and reports either
`OK` or a structured error. It is useful for CI checks and for debugging
import conflicts without launching a simulation.

---

## GUI Integration

The Nexus GUI config editor exposes the module system at three points:

- **Module browser:** A panel listing available stdlib and `NEXUS_MODULE_PATH`
  modules by category, mirroring `nexus modules list`.
- **Use-list editor:** The top-level config section includes an editor for the
  `use` array, allowing modules to be added and removed without hand-editing TOML.
- **Profile picker:** Each node's editor includes a profile selector that shows
  profiles contributed by the currently imported modules, with support for
  ordering multiple profiles per node.

---

## Design Decisions

### Why not TOML native includes?

TOML has no include mechanism. Alternatives such as pre-processing with envsubst
or m4, or adopting a different format (JSON Schema `$ref`, Dhall, CUE), would
add tooling dependencies and break the simplicity of "one TOML file." The `use`
directive is handled at the application layer and keeps the file format standard
TOML throughout.

### Why separate profiles from nodes?

Nodes require protocols (user-specific code paths), deployments (positions and
charge values), and simulation-specific parameters. These cannot be meaningfully
shared across projects. Profiles capture the *hardware characteristics* --
resources, power states, passive sinks and sources -- that are inherent to a
board or chip and reusable without modification. Keeping them separate avoids the
complexity of "node inheritance" and partial node merging.

### Why user-wins merge instead of error-on-conflict?

For the profile-to-node merge, user-wins is the natural semantic: the user is
explicitly overriding a profile field (for example, reducing the clock rate for
a low-power application). For module-to-module merges (two modules both defining
a link named `lora_915`), an error is the right behavior because the user likely
did not intend the collision and cannot see both definitions without inspecting
both files. The user's own `nexus.toml` overrides modules because it is the most
proximate expression of intent.

### Why not parameterized modules?

A parameterized module system (for example,
`use = [{ module = "lora/sx1276", frequency = 915 }]`) adds significant
complexity: template variables, type checking, and default-value semantics. The
simpler approach -- concrete variant files (`sx1276_915mhz.toml` vs
`sx1276_868mhz.toml`) with user inline overrides for one-off adjustments --
covers the common cases with far less machinery. If parameterization proves
necessary, it can be added as a backward-compatible extension later.

### Why not allow `params` or `nodes` in modules?

`params` are simulation-global (seed, timestep, root directory) and have no
meaning outside a specific simulation run. `nodes` require protocols with
filesystem paths to user firmware, making them inherently project-specific.
Restricting modules to links, channels, and profiles keeps them genuinely
portable across projects and users. The parser enforces this with
`deny_unknown_fields` on `ModuleFile`, so any attempt to include `params` or
`nodes` in a module file produces a clear parse error.
