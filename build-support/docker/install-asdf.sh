#!/usr/bin/env bash

set -e

. /build-support/shell/common/log.sh


if [ -z "${USERNAME}" ]
then
    error "Must set USERNAME environment variable"
    exit 1
fi

info "Installing asdf dependencies"
DEBIAN_FRONTEND=noninteractive apt install -y curl git

info "Fetching latest asdf release"
ASDF_DOWNLOAD_URL=$(curl -fsSL "https://api.github.com/repos/asdf-vm/asdf/releases/latest" \
    | grep "browser_download_url" \
    | grep "linux-arm64.tar.gz" \
    | grep -v '.md5' \
    | sed 's/.*"browser_download_url": "\(.*\)".*/\1/')

info "Installing asdf to /usr/local/bin"
debug "${ASDF_DOWNLOAD_URL}"
curl -fsSL "${ASDF_DOWNLOAD_URL}" | tar -xz -C /usr/local/bin asdf

info "Adding asdf shims to PATH for ${USERNAME}"
sudo -Hiu "${USERNAME}" bash -c 'echo "export PATH=\"\${HOME}/.asdf/shims:\${PATH}\"" >> "${HOME}/.bashrc"'
