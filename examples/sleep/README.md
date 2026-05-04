# Sleep

Demonstrates both variants of the sleep control file:

- `ctl.sleep.relative/<unit>` — write `N`, block until simulated time
  advances by `N` of the chosen unit.
- `ctl.sleep.absolute/<unit>` — write `T`, block until simulated time
  *reaches* `T` of the chosen unit. If `T` is already past, returns
  immediately.

Each iteration the protocol prints `<elapsed_us>,<phase>` lines at
three points: just before the relative sleep, just after, and just
after a subsequent absolute sleep. The deltas should match the
requested durations within a few timesteps.

## Running

```sh
just cli && cd examples/sleep
$NEXUS_BIN -d file simulate ./nexus.toml
```

Then inspect the captured stdout in `~/simulations/<timestamp>/output.csv`.
Expected per-iteration pattern (5 iterations, 1 ms timestep, 50 ms
relative + 25 ms absolute per loop):

```
elapsed_us,phase
<t0>,start
<t0+50000>,after_relative
<t0+75000>,after_absolute
...
```

## Status

This example is currently broken pending fixes to the sleep
implementation in commit `452fdc4`:

1. `ctrl_files.rs::parse()` maps `elapsed/*` to `Self::Time(...)`
   instead of `Self::Elapsed(...)`.
2. `fs.rs::add_processes` mounts both sleep variants under `ctl.sleep/`
   instead of `ctl.sleep.relative/` and `ctl.sleep.absolute/`.
3. The two sleep variants collide on the same FS paths; only the first
   (relative) variant is registered.
4. `sleep_alarms` is a max-heap on timestep, so `send_wakeups` fires
   the latest deadline first instead of the earliest.
5. `request_sleep` returns 1 byte written instead of `data.len()`,
   causing glibc to retry the write.

Once those are addressed, this example doubles as an end-to-end
regression test: if it produces the expected interval pattern above,
all five paths are working.
