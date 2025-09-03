#!/usr/bin/sh
cargo b --release
systemd-run --user --scope -p "Delegate=true" ./target/release/cli "$@"
