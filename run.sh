#!/usr/bin/env bash
set -e

cargo build --release
"./target/release/cargo-forgen" forgen "$@"
