#!/bin/bash
set -e

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
LUDUSAVI_SHA256="7322ff45d41eae7ae064a80d8c9ecccc5b8fb6fc090a603a66369cd4b054068d"
curl -fsSL "https://github.com/mtkennerly/ludusavi/releases/download/${LUDUSAVI_VERSION}/ludusavi-${LUDUSAVI_VERSION}-linux.tar.gz" -o /tmp/ludusavi.tar.gz
echo "${LUDUSAVI_SHA256}  /tmp/ludusavi.tar.gz" | sha256sum -c -
tar -xzf /tmp/ludusavi.tar.gz -C out
chmod +x out/ludusavi

echo " --- Cleaning up ---"
# remove root-owned target folder
cargo clean
