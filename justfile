# Default recipe
default: gui

# Build all crates in release mode
build:
    cargo build --release

# Run the CLI simulator with cgroup delegation
cli *args:
    cargo build --release -p cli
    RUST_LOG="kernel=debug" systemd-run --user --scope -p "Delegate=true" ./target/release/cli {{args}}

# Run the GUI with cgroup delegation
gui *args:
    cargo build --release -p gui
    RUST_LOG="kernel=debug" systemd-run --user --scope -p "Delegate=true" ./target/release/nexus-gui {{args}}
