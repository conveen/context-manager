# Tool to use for building and running the build container
CONTAINER_RUNTIME="docker"

# Default command to run
DEFAULT_COMMAND="build"

# Default Rust version to install in build container
DEFAULT_RUST_VERSION="stable"

# Default runtime context for commands
# container: run commands in build container
# local: run commands on host system
# This option should not be changed from "container" without good reason
# Can be overriden at runtime with CLI parameter
RUNTIME_CONTEXT="container"

# Default container registry to push build container image to
BUILD_IMAGE_REGISTRY="ghcr.io"
# URL of the build container image, including the registry hostname and image path
BUILD_IMAGE_URL="${BUILD_IMAGE_REGISTRY}/conveen/$(basename $(pwd))"
# Tag of the build container image
BUILD_IMAGE_TAG="build"
# Target stage of the build container image
BUILD_TARGET_STAGE="build"
# URL of the release container image, including the registry hostname and image path
RELEASE_IMAGE_URL="${BUILD_IMAGE_URL}"
# Target stage of the release container image
RELEASE_TARGET_STAGE="release"
# Tag of the release container image
RELEASE_IMAGE_TAG="release"

# Username and UID of executing user
USERID="$(id -u)"
USERNAME="$(id -un)"
# Docker group GID of host
DOCKER_GID="$(! test -z $(which getent) && ( getent group docker | cut -d: -f3 ) || id -g)"
