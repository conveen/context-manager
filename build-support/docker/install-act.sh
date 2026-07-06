#!/usr/bin/env bash

set -e

. /build-support/shell/common/log.sh


if [ -z "${USERNAME}" ]
then
    error "Must set USERNAME environment variable"
    exit 1
fi

# Install latest act CLI from GitHub releases
sudo -Hiu $USERNAME bash -c 'curl -sSf https://raw.githubusercontent.com/nektos/act/master/install.sh > /tmp/install-act.sh'
chmod 755 /tmp/install-act.sh
/tmp/install-act.sh
rm /tmp/install-act.sh
