#!/usr/bin/env bash

set -e

. /build-support/shell/common/log.sh


if [ -z "${USERNAME}" ]
then
    error "Must set USERNAME environment variable"
    exit 1
fi


apt install -y \
    build-essential \
    make \
    libssl-dev \
    pkg-config \
    python3 \
    python3-pip

sudo -Hiu "${USERNAME}" bash -c 'asdf plugin add nodejs https://github.com/asdf-vm/asdf-nodejs.git'
# Version must match .tool-versions
sudo -Hiu "${USERNAME}" bash -c 'asdf install nodejs lts && asdf set --home nodejs lts'
