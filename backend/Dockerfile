FROM ghcr.io/steamdeckhomebrew/holo-toolchain-rust:latest

# Install required packages
RUN pacman -Sy --noconfirm \
    openssl \
    openssl-1.1 \
    pkgconf

# Entry point
ENTRYPOINT ["/backend/entrypoint.sh"]
