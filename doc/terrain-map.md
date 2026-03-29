# Terrain & Map Integration — Research and Implementation Plan

## Table of Contents

1. [Overview](#overview)
2. [Research Findings](#research-findings)
   - [Signal Propagation Pipeline](#signal-propagation-pipeline)
   - [Distance & RSSI Call Chain](#distance--rssi-call-chain)
   - [GUI Rendering Pipeline](#gui-rendering-pipeline)
   - [Config Parsing Pipeline](#config-parsing-pipeline)
3. [Design](#design)
   - [Phase 1: 2D Terrain with Obstacles](#phase-1-2d-terrain-with-obstacles)
   - [Phase 2: 3D Topology and GUI Overlay](#phase-2-3d-topology-and-gui-overlay)
4. [Configuration Schema](#configuration-schema)
5. [Implementation Plan](#implementation-plan)
   - [Phase 1 File-by-File Changes](#phase-1-file-by-file-changes)
   - [Phase 2 File-by-File Changes](#phase-2-file-by-file-changes)
6. [Injection Points](#injection-points)
7. [Performance Considerations](#performance-considerations)
8. [Test Plan](#test-plan)

---

## Overview

This document captures all research findings and the implementation plan for
adding terrain and map support to Nexus. The feature has two phases:

- **Phase 1 (2D terrain + signal obstruction):** Import a 2D terrain/obstacle
  map that affects signal propagation. Obstacles between nodes introduce
  additional attenuation beyond free-space path loss. Nodes and their
  orientations are placed on the map. The kernel uses the terrain data during
  RSSI computation to model non-line-of-sight (NLOS) effects.

- **Phase 2 (3D topology + GUI overlay):** Extend to 3D elevation data
  (heightmaps). Render the terrain/map as a background layer in the GUI grid
  view. Overlay node positions with altitude awareness. Support topographic
  data for elevation-dependent signal propagation.

---

## Research Findings

### Signal Propagation Pipeline

The current RSSI computation flows through these functions, in order:

```
Position::distance(src, dst) → (f64, DistanceUnit)
    ↓
RssiProbExpr::rssi(tx_power, distance, unit, medium) → f64
    ↓
Medium::rssi(tx_power, distance_meters) → f64
    ├─ Wireless: rssi_wireless() — Friis free-space: Pt + G - 20log₁₀(4πd/λ)
    └─ Wired:    rssi_wired()   — RLGC: Pt - 8.686·α·d
```

**Key files and signatures:**

| File | Function | Signature |
|------|----------|-----------|
| `config/src/position.rs` | `Position::distance` | `fn distance(from: &Self, to: &Self) -> (f64, DistanceUnit)` |
| `config/src/medium.rs` | `Medium::rssi` | `fn rssi(&self, tx_power_dbm: f64, distance_meters: f64) -> f64` |
| `config/src/medium.rs` | `rssi_wireless` | Private, applies Friis model |
| `config/src/medium.rs` | `rssi_wired` | Private, applies RLGC model |
| `config/src/medium.rs` | `RssiProbExpr::rssi` | `fn rssi(&self, tx_power_dbm: f64, distance: f64, unit: DistanceUnit, medium: &Medium) -> f64` |
| `config/src/medium.rs` | `RssiProbExpr::probability` | `fn probability(&self, rssi: f64) -> f64` — evaluates meval expression with `rssi` and `snr` variables |

**Critical observation:** `Medium::rssi()` takes only `tx_power_dbm` and
`distance_meters`. It has no access to the source/destination positions
themselves. To inject terrain loss, we must either:

1. Add a `terrain_loss_db: f64` parameter to `Medium::rssi()`, or
2. Subtract terrain loss after `Medium::rssi()` returns, in the caller
   (`RssiProbExpr::rssi` or `send_through_channel`).

Option 2 is cleaner — it keeps the medium model pure (physics of the medium)
and adds terrain as a separate, composable loss factor.

### Distance & RSSI Call Chain

RSSI is consumed at two points in message delivery:

#### Exclusive channels — at queue time (`delivery.rs`)

```rust
// In queue_message():
let (distance, unit) = Position::distance(&src_node.position, &dst_node.position);
let result = send_through_channel(channel, buf, distance, unit, rng);
//           ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
//           This is where terrain_loss would be injected
```

#### Shared channels — at delivery time (`delivery.rs`)

```rust
// In deliver_shared_msg():
let (distance, unit) = Position::distance(&src_node.position, &dst_node.position);
let result = send_through_channel(channel, buf, distance, unit, rng);
```

Both paths call `Position::distance()` then `send_through_channel()`. The
positions of both endpoints are available at both call sites, which is exactly
what we need for ray-casting through a terrain map.

**`send_through_channel` signature** (`link_simulation.rs`):

```rust
pub(super) fn send_through_channel<'a>(
    channel: &Channel,
    mut buf: Cow<'a, [u8]>,
    distance: f64,
    unit: DistanceUnit,
    rng: &mut StdRng,
) -> Option<(Cow<'a, [u8]>, bool, f64, f64)>
```

Adding `terrain_loss_db: f64` here is the simplest injection point.

### GUI Rendering Pipeline

**GridView** (`gui/src/render/grid.rs`):

```rust
pub struct GridView {
    pub offset: Vec2,    // pan offset in screen pixels
    pub zoom: f32,       // world-to-screen scale factor
}
```

Key methods:
- `world_to_screen(world: Pos2, canvas_rect: Rect) -> Pos2`
- `screen_to_world(screen: Pos2, canvas_rect: Rect) -> Pos2`

Y-axis is inverted: world Y increases upward, screen Y increases downward.

**Rendering order in `show_grid_panel`** (`gui/src/panels/grid.rs`):

1. Canvas allocation + scrollbar handling
2. Input processing (pan/zoom)
3. **→ INSERT TERRAIN BACKGROUND HERE ←**
4. Grid line drawing (`grid.draw()`)
5. Node drawing (circles + labels)
6. Message arc animations
7. Hit testing

**egui texture support:**
- `ctx.load_texture(name, ColorImage)` → `TextureHandle`
- `painter.image(texture_id, rect, uv, tint)` — draws a texture in a rect
- The painter is obtained via `ui.painter_at(canvas_rect)`

A terrain image would be loaded once, cached as a `TextureHandle`, and drawn
before grid lines using `world_to_screen()` to map terrain bounds to screen
coordinates.

### Config Parsing Pipeline

The config parse pipeline is strictly three-phase:

```
TOML text → parse::Simulation (serde) → validate → ast::Simulation (runtime)
```

All parse structs use `#[serde(default, deny_unknown_fields)]`. Adding a new
`[terrain]` section requires:

1. Add `terrain: Option<Terrain>` to `parse::Simulation` in `config/src/parse.rs`
2. Add `terrain: Option<Terrain>` to `ast::Simulation` in `config/src/ast.rs`
3. Add `Terrain::validate()` in `config/src/validate/mod.rs`
4. Call validation from `Simulation::validate()`

**The `Medium` enum** is a tagged enum (`#[serde(tag = "type")]`) with
`Wireless` and `Wired` variants. Terrain is NOT a medium type — it's a
separate layer that applies additional attenuation on top of any medium.

**meval expressions** already support `rssi` and `snr` as variables. Adding a
`terrain_loss` variable to the evaluation context would let users write
expressions like `"max(0.0, (rssi + terrain_loss + 120.0) / 120.0)"`.

---

## Design

### Phase 1: 2D Terrain with Obstacles

**Concept:** A 2D obstacle map defines rectangular or polygonal regions with
material properties. When computing RSSI between two nodes, a ray is cast
from source to destination. For each obstacle the ray intersects, an
attenuation value (in dB) is accumulated. This total terrain loss is
subtracted from the free-space RSSI.

**Data model:**

```toml
[terrain]
unit = "m"                           # coordinate unit for obstacle definitions

[[terrain.obstacles]]
name = "building_a"
material = "concrete"                # predefined material or custom
attenuation_db = 15.0                # dB loss per traversal (override)
shape = { type = "rect", min = [10, 20], max = [30, 50] }

[[terrain.obstacles]]
name = "forest"
material = "foliage"
attenuation_db = 6.0
shape = { type = "rect", min = [100, 0], max = [200, 80] }

[[terrain.obstacles]]
name = "wall"
material = "concrete"
shape = { type = "line", start = [50, 0], end = [50, 100], thickness = 0.3 }
```

**Predefined materials (default attenuation_db per traversal):**

| Material | Attenuation (dB) | Source |
|----------|-----------------|--------|
| `concrete` | 15 | ITU-R P.2109 |
| `brick` | 10 | ITU-R P.2109 |
| `wood` | 4 | ITU-R P.2109 |
| `glass` | 3 | ITU-R P.2109 |
| `foliage` | 6 | Empirical 2.4 GHz |
| `metal` | 25 | ITU-R P.2109 |
| `drywall` | 3 | Empirical |
| `water` | 20 | Empirical |

**Ray-cast algorithm:**

For each sender→receiver pair:

1. Project both positions onto the XY plane (2D).
2. Cast a line segment from sender to receiver.
3. For each obstacle, test intersection with the shape.
4. Accumulate `attenuation_db` for each intersected obstacle.
5. Return total terrain loss in dB.

**Obstacle shapes (Phase 1 — 2D only):**

- **Rect:** Axis-aligned rectangle defined by `min` and `max` corners. Ray
  tests all four edges; each crossing adds attenuation once.
- **Line:** A thin wall defined by `start`, `end`, and `thickness`. Ray tests
  a single line segment; crossing adds attenuation once.

**Integration with RSSI:**

```
final_rssi = Medium::rssi(tx_power, distance) - terrain_loss_db
```

The `terrain_loss_db` is also exposed as a variable in meval expressions:

```rust
ctx.var("terrain_loss", terrain_loss_db);
```

This allows users to write custom packet_loss or bit_error expressions that
reference terrain effects.

### Phase 2: 3D Topology and GUI Overlay

**Concept:** Extend to 3D with a heightmap (grid of elevation values).
Render the terrain as a background image in the GUI. Nodes are placed at
their (x, y) position with altitude from the heightmap or from their
configured z coordinate.

**Heightmap data model:**

```toml
[terrain]
unit = "m"
heightmap = "terrain.png"            # grayscale PNG, pixel value → elevation
heightmap_bounds = { min = [0, 0], max = [1000, 1000] }
elevation_range = [0, 500]           # min/max elevation mapped from pixel values

# Optional background image for GUI overlay
map_image = "map.png"
map_bounds = { min = [0, 0], max = [1000, 1000] }
```

**3D ray-cast:** Sample the heightmap along the 2D ray between sender and
receiver. If any sample point has elevation above the line connecting sender
altitude to receiver altitude, the terrain blocks line-of-sight. Additional
knife-edge diffraction loss can be computed from the Fresnel zone obstruction.

**GUI overlay:** The `map_image` is loaded as an egui texture and drawn as
the first layer in `show_grid_panel`, before grid lines and nodes. The
`map_bounds` define the world-space extent; `world_to_screen()` maps it to
screen coordinates.

---

## Configuration Schema

### Phase 1 TOML

```toml
[terrain]
unit = "m"                           # distance unit for all terrain coordinates

# Predefined material attenuation can be overridden
[terrain.materials]
concrete = 15.0                      # dB per traversal
custom_wall = 8.0                    # user-defined material

[[terrain.obstacles]]
name = "office_building"             # human-readable label
material = "concrete"                # references terrain.materials or built-in
shape = { type = "rect", min = [10, 20], max = [30, 50] }

[[terrain.obstacles]]
name = "partition_wall"
attenuation_db = 5.0                 # inline override (no material reference)
shape = { type = "line", start = [50, 0], end = [50, 100], thickness = 0.3 }
```

### Phase 2 TOML additions

```toml
[terrain]
# ... phase 1 fields ...
heightmap = "terrain.png"
heightmap_bounds = { min = [0, 0], max = [1000, 1000] }
elevation_range = [0.0, 500.0]
map_image = "satellite.png"
map_bounds = { min = [0, 0], max = [1000, 1000] }
map_opacity = 0.6
```

---

## Implementation Plan

### Phase 1 File-by-File Changes

#### 1. `config/src/ast.rs` — New types

```rust
pub struct Terrain {
    pub unit: DistanceUnit,
    pub materials: HashMap<String, f64>,          // name → dB
    pub obstacles: Vec<Obstacle>,
}

pub struct Obstacle {
    pub name: String,
    pub attenuation_db: f64,
    pub shape: ObstacleShape,
}

pub enum ObstacleShape {
    Rect { min: [f64; 2], max: [f64; 2] },
    Line { start: [f64; 2], end: [f64; 2], thickness: f64 },
}
```

Add `pub terrain: Option<Terrain>` to `ast::Simulation`.

#### 2. `config/src/parse.rs` — Deserialization

Mirror the AST types with `Option` fields, `deny_unknown_fields`, and serde
tagged enum for `ObstacleShape`. Add `terrain: Option<Terrain>` to
`parse::Simulation`.

#### 3. `config/src/validate/mod.rs` — Validation

`Terrain::validate()`:
- Verify `unit` is a valid `DistanceUnit`
- Verify each obstacle has either `material` (referencing a known material) or
  inline `attenuation_db`
- Verify shape coordinates are valid (min < max for rects, start ≠ end for lines)
- Resolve material references to dB values

Call from `Simulation::validate()`.

#### 4. New file: `config/src/terrain.rs` — Ray-cast logic

```rust
pub struct TerrainMap {
    obstacles: Vec<ResolvedObstacle>,
    unit: DistanceUnit,
}

impl TerrainMap {
    pub fn from_config(terrain: &Terrain) -> Self { ... }

    /// Compute total terrain attenuation in dB between two 2D points.
    pub fn attenuation_db(&self, from: (f64, f64), to: (f64, f64)) -> f64 {
        let mut total = 0.0;
        for obs in &self.obstacles {
            if obs.shape.intersects_segment(from, to) {
                total += obs.attenuation_db;
            }
        }
        total
    }
}
```

Ray-segment intersection for `Rect` uses slab method (check X and Y slabs).
Ray-segment intersection for `Line` uses 2D segment-segment intersection.

#### 5. `kernel/src/types.rs` — Store terrain in kernel

Add `terrain: Option<TerrainMap>` to the resolved kernel state. Passed through
from config at initialization.

#### 6. `kernel/src/router/link_simulation.rs` — Apply terrain loss

Add `terrain_loss_db: f64` parameter to `send_through_channel`:

```rust
pub(super) fn send_through_channel<'a>(
    channel: &Channel,
    buf: Cow<'a, [u8]>,
    distance: f64,
    unit: DistanceUnit,
    terrain_loss_db: f64,       // NEW
    rng: &mut StdRng,
) -> Option<(Cow<'a, [u8]>, bool, f64, f64)>
```

Inside: subtract `terrain_loss_db` from the computed RSSI before packet loss
and bit error evaluation.

#### 7. `kernel/src/router/delivery.rs` — Compute terrain loss at call sites

At both exclusive (queue_message) and shared (deliver_shared_msg) call sites:

```rust
let terrain_loss = terrain.as_ref()
    .map(|t| t.attenuation_db(
        (src.position.point.x, src.position.point.y),
        (dst.position.point.x, dst.position.point.y),
    ))
    .unwrap_or(0.0);
let result = send_through_channel(channel, buf, distance, unit, terrain_loss, rng);
```

#### 8. `config/src/medium.rs` — Add `terrain_loss` to meval context

In `RssiProbExpr::probability()`, add:

```rust
ctx.var("terrain_loss", terrain_loss_db);
```

This requires threading `terrain_loss_db` through to the probability call.

#### 9. `kernel/src/resolver.rs` — Pass terrain through

Convert `ast::Terrain` → `TerrainMap` during kernel initialization and store
it in the routing server.

### Phase 2 File-by-File Changes

#### 10. `config/src/ast.rs` — Heightmap and map fields

Add to `Terrain`:

```rust
pub heightmap: Option<HeightmapConfig>,
pub map_image: Option<MapImageConfig>,
```

#### 11. New file: `config/src/heightmap.rs` — Heightmap loading

Load grayscale PNG, map pixel values to elevation range, provide
`elevation_at(x, y)` and `profile(from, to, samples)` methods.

#### 12. `gui/src/state.rs` — Terrain overlay state

Add `TerrainOverlay` to `ConfigEditorState`, `LiveSimState`, `ReplayState`:

```rust
pub struct TerrainOverlay {
    pub texture: Option<TextureHandle>,
    pub world_bounds: (Pos2, Pos2),
    pub opacity: f32,
    pub enabled: bool,
}
```

#### 13. `gui/src/panels/grid.rs` — Draw terrain background

Insert terrain rendering between input handling and grid drawing:

```rust
if let Some(overlay) = &terrain_overlay {
    if overlay.enabled {
        let min = grid.world_to_screen(overlay.world_bounds.0, canvas_rect);
        let max = grid.world_to_screen(overlay.world_bounds.1, canvas_rect);
        let rect = Rect::from_min_max(min, max);
        let tint = Color32::from_white_alpha((overlay.opacity * 255.0) as u8);
        painter.image(overlay.texture.id(), rect, Rect::from_min_max(pos2(0.0,0.0), pos2(1.0,1.0)), tint);
    }
}
```

#### 14. `gui/src/render/` — Obstacle outlines

Draw obstacle shapes as semi-transparent colored rectangles/lines on the grid
view so users can see where obstacles are.

---

## Injection Points

Summary of where terrain touches existing code:

| File | What changes | Why |
|------|-------------|-----|
| `config/src/ast.rs` | Add `Terrain`, `Obstacle`, `ObstacleShape` types; add `terrain` field to `Simulation` | Data model |
| `config/src/parse.rs` | Add serde deserialization structs | TOML parsing |
| `config/src/validate/mod.rs` | Add `Terrain::validate()` | Config validation |
| `config/src/terrain.rs` | **New file**: `TerrainMap`, ray-cast logic | Core algorithm |
| `config/src/medium.rs` | Add `terrain_loss` to meval context in `probability()` | Expression variable |
| `kernel/src/types.rs` | Store `Option<TerrainMap>` | Kernel state |
| `kernel/src/resolver.rs` | Build `TerrainMap` from config | Initialization |
| `kernel/src/router/link_simulation.rs` | Add `terrain_loss_db` param to `send_through_channel` | RSSI adjustment |
| `kernel/src/router/delivery.rs` | Compute terrain loss at queue/delivery time | Call site |
| `gui/src/state.rs` | Add `TerrainOverlay` state | GUI state (Phase 2) |
| `gui/src/panels/grid.rs` | Draw terrain background | Rendering (Phase 2) |

---

## Performance Considerations

**N² ray-cast cost:** For N nodes, each message requires one ray-cast per
destination. With M obstacles, each ray-cast is O(M) segment intersection
tests. Total per-timestep cost for a fully-connected network: O(N² × M).

**Mitigations:**

1. **Spatial indexing:** Build a 2D grid index of obstacles. Only test
   obstacles whose bounding boxes overlap the sender→receiver bounding box.
   Reduces per-ray cost from O(M) to O(k) where k << M.

2. **Attenuation cache:** For static nodes, cache the terrain loss between
   each node pair. Invalidate on position change (mobile nodes). Cache key:
   `(src_node_idx, dst_node_idx)` → `terrain_loss_db`.

3. **Lazy evaluation:** Only compute terrain loss for wireless links. Wired
   links (cables) are not affected by terrain.

4. **Heightmap sampling:** For Phase 2, sample the heightmap at fixed intervals
   along the ray (e.g., every 10 meters) rather than per-pixel. 50 samples
   per ray is sufficient for most topography.

**Estimated overhead for Phase 1:** For 20 nodes and 100 obstacles without
spatial indexing: 400 rays × 100 tests = 40,000 segment intersections per
timestep. At ~10ns per intersection, this is ~0.4ms — negligible compared to
process scheduling overhead.

---

## Test Plan

### Unit tests (config/src/terrain.rs)

1. `test_rect_intersects_segment` — ray through a rectangle
2. `test_rect_misses_segment` — ray that misses a rectangle
3. `test_rect_tangent_segment` — ray along edge of rectangle
4. `test_line_intersects_segment` — ray crossing a wall
5. `test_line_misses_segment` — ray parallel to wall
6. `test_line_thickness` — ray through thick wall
7. `test_multiple_obstacles` — ray through two obstacles accumulates attenuation
8. `test_zero_obstacles` — empty terrain returns 0.0 dB loss
9. `test_same_point` — sender and receiver at same position returns 0.0
10. `test_unit_conversion` — terrain in km, nodes in m → coordinates converted

### Integration tests (config validation)

11. `test_parse_terrain_config` — valid TOML with terrain section parses
12. `test_parse_terrain_no_obstacles` — terrain section with no obstacles is valid
13. `test_parse_terrain_bad_material` — unknown material name is rejected
14. `test_parse_terrain_bad_shape` — invalid shape coordinates rejected
15. `test_parse_terrain_rect_min_max` — min > max rejected

### Kernel integration tests

16. `test_terrain_reduces_rssi` — message through obstacle has lower RSSI
17. `test_terrain_causes_packet_loss` — high attenuation causes drops
18. `test_terrain_no_effect_on_wired` — wired links ignore terrain
19. `test_terrain_loss_in_expression` — `terrain_loss` variable in meval works
20. `test_terrain_with_mobile_nodes` — moving node changes terrain intersection
<<<<<<< HEAD

---

## Phase 3: Real Terrain File Import and GUI Node Placement

### Motivation

Users should be able to download a terrain map of a real location (e.g.,
Mount Mansfield, Vermont) and import it directly into Nexus. The terrain
displays in the GUI as a background layer, and users can drag-and-drop nodes
onto the map. Node coordinates (x, y, z) are automatically derived from
where the node is placed on the terrain overlay — including elevation from
the heightmap data.

### Supported File Formats

#### Heightmap / Elevation Data

| Format | Extension | Description | Rust Support |
|--------|-----------|-------------|-------------|
| **PNG heightmap** | `.png` | Grayscale image, pixel value → elevation | `image` crate (mature) |
| **GeoTIFF** | `.tif`, `.tiff` | Georeferenced raster with embedded CRS/bounds | `tiff` crate for reading; metadata via `gdal` bindings or manual tag parsing |
| **SRTM HGT** | `.hgt` | NASA Shuttle Radar Topography Mission. Raw 16-bit signed big-endian grid. 1-arc-second (3601×3601) or 3-arc-second (1201×1201) | Simple binary parse — no crate needed |
| **USGS DEM** | `.dem` | ASCII grid with header. Legacy format. | Simple text parse |

**Primary target: PNG heightmap + sidecar metadata.** This is the simplest
to implement and covers the majority of use cases. GeoTIFF support can be
added later as an extension.

**Sidecar metadata format** (for PNG):
```toml
[terrain]
heightmap = "mount_mansfield.png"
heightmap_bounds = { min = [-72.83, 44.50], max = [-72.76, 44.56] }  # lon/lat or local XY
elevation_range = [300.0, 1339.0]  # meters: min elevation, max elevation
coordinate_system = "local"  # "local" (XY meters) or "wgs84" (lon/lat → auto-project)
unit = "m"
```

When `coordinate_system = "wgs84"`, Nexus automatically projects lat/lon
bounds into local meters using a simple equirectangular projection centered
on the map. This is sufficient for areas up to ~50 km where UTM distortion
is negligible.

#### Visual Map Overlay

| Format | Extension | Description |
|--------|-----------|-------------|
| **PNG/JPEG** | `.png`, `.jpg` | Satellite imagery, topographic map, or any raster image |
| **SVG** | `.svg` | Vector floor plan or campus map |

```toml
[terrain]
map_image = "satellite.png"           # visual overlay for GUI
map_bounds = { min = [0, 0], max = [1000, 1000] }  # world-space extent
map_opacity = 0.6                     # transparency (0.0 = invisible, 1.0 = opaque)
```

The `map_image` is purely visual — it does not affect signal propagation.
The `heightmap` provides elevation data and can also be rendered as a
tinted background if no separate `map_image` is provided.

#### Vector Obstacles (future)

| Format | Extension | Description |
|--------|-----------|-------------|
| **GeoJSON** | `.geojson` | Buildings, walls, forests as polygons |
| **Shapefile** | `.shp` | Common GIS format for building footprints |

These would be imported as obstacle definitions (extending the existing
`[[terrain.obstacles]]` system). Not in initial implementation.

### Heightmap Data Model

```rust
/// A loaded heightmap with elevation data and coordinate mapping.
pub struct Heightmap {
    /// Raw elevation grid (row-major, top-to-bottom).
    data: Vec<f32>,
    /// Grid dimensions.
    width: usize,
    height: usize,
    /// World-space bounds of the heightmap.
    bounds_min: (f64, f64),
    bounds_max: (f64, f64),
    /// Elevation range in the terrain's distance unit.
    elevation_min: f64,
    elevation_max: f64,
    /// The terrain distance unit (e.g., meters).
    unit: DistanceUnit,
}

impl Heightmap {
    /// Load from a PNG file + metadata.
    pub fn from_png(path: &Path, bounds: ..., elevation_range: ...) -> Result<Self>;

    /// Load from an SRTM HGT file.
    pub fn from_hgt(path: &Path) -> Result<Self>;

    /// Get the elevation at a world-space (x, y) coordinate.
    /// Returns None if the point is outside the heightmap bounds.
    pub fn elevation_at(&self, x: f64, y: f64) -> Option<f64>;

    /// Sample elevations along a line from (x0,y0) to (x1,y1).
    /// Returns a vec of (distance_along_line, elevation) pairs.
    pub fn profile(&self, x0: f64, y0: f64, x1: f64, y1: f64, samples: usize) -> Vec<(f64, f64)>;

    /// Convert the heightmap to an egui-compatible RGBA image for rendering.
    pub fn to_color_image(&self, colormap: &Colormap) -> egui::ColorImage;
}
```

### GUI Integration: Terrain Overlay and Node Drag-Drop

#### Rendering Pipeline (in `show_grid_panel`)

The terrain overlay is drawn as the first visual layer, before grid lines
and nodes:

```
1. Allocate canvas + handle scrollbars
2. Handle pan/zoom input
3. ★ Draw terrain map image (background layer)
4. ★ Draw obstacle outlines (semi-transparent colored shapes)
5. Draw grid lines
6. Draw nodes
7. Draw message arcs
8. Hit-test nodes
```

#### Terrain Texture Management

```rust
// In ConfigEditorState / LiveSimState / ReplayState
pub struct TerrainOverlay {
    /// Loaded heightmap data (for elevation queries).
    pub heightmap: Option<Heightmap>,
    /// Cached egui texture handle for the map image.
    pub map_texture: Option<egui::TextureHandle>,
    /// World-space bounds of the map overlay.
    pub world_bounds: (Pos2, Pos2),
    /// Opacity for the map overlay (0.0–1.0).
    pub opacity: f32,
    /// Whether the overlay is visible.
    pub enabled: bool,
}
```

The texture is loaded once when the terrain config is parsed or when a file
is opened via the GUI file picker. It is cached as an `egui::TextureHandle`
and re-rendered each frame using `Painter::image()`.

#### Node Drag-and-Drop Placement

In the **Config Editor** mode, nodes can be placed on the map by:

1. **Click to place**: Click on the terrain map to set a node's position.
   The clicked screen coordinate is converted to world-space via
   `grid.screen_to_world()`. If a heightmap is loaded, the Z coordinate
   is automatically set to `heightmap.elevation_at(x, y)`.

2. **Drag to move**: Drag an existing node on the grid. The node's position
   updates in real-time. On drag release, the final (x, y, z) is written
   back to the node's deployment config.

**Implementation in `show_grid_panel` (Config Editor mode):**

```rust
// Detect drag-on-node in config editor
if response.dragged_by(PointerButton::Primary) && drag_started_on_node {
    if let Some(pointer_pos) = response.interact_pointer_pos() {
        let world_pos = grid.screen_to_world(pointer_pos, canvas_rect);
        if let Some(selected) = selected_node {
            // Update the node's position in the simulation config
            if let Some(node) = sim.nodes.get_mut(selected) {
                node.position.point.x = world_pos.x as f64;
                node.position.point.y = world_pos.y as f64;
                // Auto-derive Z from heightmap
                if let Some(ref hm) = terrain_overlay.heightmap {
                    if let Some(z) = hm.elevation_at(world_pos.x as f64, world_pos.y as f64) {
                        node.position.point.z = z;
                    }
                }
            }
        }
    }
}
```

3. **Right-click context menu**: Right-click on the map to create a new node
   at that position. A dialog asks for the node name and protocol config.

4. **Elevation display**: When hovering over the terrain, display the
   elevation at the cursor position in the status bar or tooltip.

#### File Picker Integration

The Config Editor toolbar includes a "Load Terrain" button that opens a
native file dialog (via the `rfd` crate, already a dependency of the GUI):

```rust
if ui.button("Load Terrain").clicked() {
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("Terrain files", &["png", "tif", "tiff", "hgt"])
        .pick_file()
    {
        // Load the file and update terrain state
        load_terrain_file(&path, &mut state.terrain_overlay, ui.ctx());
    }
}
```

For PNG heightmaps without embedded metadata, a dialog prompts the user for:
- World-space bounds (min/max XY)
- Elevation range (min/max Z)
- Distance unit

For GeoTIFF files, bounds and elevation range are extracted from the file
metadata automatically.

### Phase 3 File-by-File Changes

| File | Change | Purpose |
|------|--------|---------|
| `config/src/terrain.rs` | Add `Heightmap` struct with `from_png()`, `elevation_at()`, `profile()`, `to_color_image()` | Heightmap loading and elevation queries |
| `config/src/parse.rs` | Add `heightmap`, `heightmap_bounds`, `elevation_range`, `map_image`, `map_opacity` to `Terrain` | Config parsing for file imports |
| `config/src/validate/mod.rs` | Validate heightmap file exists, bounds are valid, elevation range is valid | Config validation |
| `gui/src/state.rs` | Add `TerrainOverlay` struct to editor/sim/replay states | GUI state |
| `gui/src/panels/grid.rs` | Draw terrain texture before grid; handle node drag-drop | Rendering + interaction |
| `gui/src/panels/toolbar.rs` | Add "Load Terrain" button with file picker | File import UI |
| `gui/src/config_editor/nodes.rs` | Show derived elevation in node position editor | Elevation feedback |
| `gui/src/render/grid.rs` | Add `draw_terrain()` method that renders texture + obstacle outlines | Rendering helpers |
| `Cargo.toml` (config) | Add `image` crate dependency | PNG/image loading |
| `Cargo.toml` (gui) | Already has `rfd` for file dialogs | No change |

### Coordinate System Handling

For imported real-world terrain (e.g., Mount Mansfield):

1. **WGS84 input**: User provides bounds as (longitude, latitude) pairs.
2. **Equirectangular projection**: Convert to local meters centered on the
   map centroid:
   ```
   x_meters = (lon - lon_center) * cos(lat_center) * 111_320
   y_meters = (lat - lat_center) * 110_540
   ```
3. **Simulation coordinates**: All internal math uses the projected (x, y)
   in meters. The heightmap bounds are converted once at load time.
4. **Display**: The GUI can optionally show lat/lon alongside XY coordinates
   when a WGS84 terrain is loaded.

This avoids requiring a full projection library (like `proj`) while being
accurate enough for areas up to ~50 km across.

### Example Workflow: Mount Mansfield

1. Download SRTM 1-arc-second data tile `N44W073.hgt` from USGS.
2. In Nexus GUI, click "Load Terrain" → select `N44W073.hgt`.
3. Dialog: "SRTM tile detected. Bounds: 44°N–45°N, 72°W–73°W. Crop to area?"
4. User crops to Mount Mansfield area (44.50°N–44.56°N, 72.76°W–72.83°W).
5. Terrain renders in the grid view as a colorized elevation map.
6. User clicks on the map to place sensor nodes at specific locations.
7. Each node's Z coordinate is auto-set from the elevation data.
8. User defines LoRa channels and runs the simulation.
9. Signal propagation accounts for terrain obstruction (Phase 1 obstacles)
   and distance (already implemented).

=======
>>>>>>> 94bf81c (feat: add terrain/map system with 2D obstacle-based signal attenuation)
