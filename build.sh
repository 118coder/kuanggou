#!/usr/bin/env bash
set -e
cargo build --release
mkdir -p dist
cp target/release/cutout_tool.exe dist/kuanggou.exe
cp target/release/cutout_tool.exe 框构.exe
echo "OK -> dist/kuanggou.exe / 框构.exe"
