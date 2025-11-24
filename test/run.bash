#!/usr/bin/env bash
set -e

cargo forgen
cargo "$@"
