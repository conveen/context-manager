#!/usr/bin/env bash

set -e

. /build-support/shell/common/log.sh


if [ -z "${USERNAME}" ]
then
    error "Must set USERNAME environment variable"
    exit 1
fi

apt install -y \
    libglib2.0-dev \
    libwebkit2gtk-4.1-dev \
    libgtk-3-dev \
    libgdk-pixbuf2.0-0 \
    libpango1.0-0 \
    libcairo2-dev \
    libatk1.0-0

info "Installing llvm-cov for creating coverage reports"
sudo -Hiu $USERNAME bash -c '$HOME/.cargo/bin/cargo install --locked cargo-llvm-cov'
