#!/bin/bash

echo "--- Rust version info ---"
rustup --version
rustc --version
cargo --version

echo "--- Building plugin backend ---"
cargo build --release
mkdir -p out
cp target/release/backend out/backend

echo "--- Downloading ludusavi ---"
LUDUSAVI_VERSION="v0.31.0"
curl -fsSL "https://github.com/mtkennerly/ludusavi/releases/download/${LUDUSAVI_VERSION}/ludusavi-${LUDUSAVI_VERSION}-linux.tar.gz" -o /tmp/ludusavi.tar.gz
tar -xzf /tmp/ludusavi.tar.gz -C out
chmod +x out/ludusavi

echo " --- Cleaning up ---"
# remove root-owned target folder
cargo clean
