#!/usr/bin/env bash

set -e

###################
##### Imports #####
###################

# Check if running in build container or locally
# to import from correct path
if [ -d /build-support ]
then
    . ${HOME}/.bashrc
    . ${HOME}/.cargo/env
    # .bashrc has an interactive-only guard so the asdf shims PATH export it
    # contains is skipped when sourced from a script. Add the shims explicitly
    # so tools installed via asdf (node, npm, …) are available here.
    [ -d "${HOME}/.asdf/shims" ] && export PATH="${HOME}/.asdf/shims:${PATH}"
    BUILD_SUPPORT_ROOT="/build-support"
else
    BUILD_SUPPORT_ROOT="./build-support"
fi
. "${BUILD_SUPPORT_ROOT}/shell/run/config.sh"
. "${BUILD_SUPPORT_ROOT}/shell/common/log.sh"


###############################
##### Container Utilities #####
###############################

run-build-base() {
    ${CONTAINER_RUNTIME} build \
        --target "${BUILD_TARGET_STAGE}" \
        -t "${BUILD_IMAGE_URL}:${BUILD_IMAGE_TAG}" \
        -f build-support/docker/Dockerfile \
        --build-arg DOCKER_GID="${DOCKER_GID}" \
        --build-arg RUST_VERSION="${DEFAULT_RUST_VERSION}" \
        --build-arg UID="${USERID}" \
        --build-arg USERNAME="${USERNAME}" \
        "${@}" \
        .
}

run-push-base() {
    ${CONTAINER_RUNTIME} push \
        "${@}" \
        "${BUILD_IMAGE_URL}:${BUILD_IMAGE_TAG}"
}

run-in-container() {
    local COMMAND="${1}"
    # If input device is not a TTY don't run with `-it` flags
    local INTERACTIVE_FLAGS="$(test -t 0 && echo '-it' || echo '')"
    ${CONTAINER_RUNTIME} run \
		--rm \
         ${INTERACTIVE_FLAGS} \
         ${PORT_FLAGS} \
		-u ${USERNAME} \
        -e "CROSS_CONTAINER_IN_CONTAINER=true" \
        -e "RUST_BACKTRACE" \
        -e "RUST_LOG" \
        -e "RUSTFLAGS" \
        -v ${HOME}/.cargo/git:/home/${USERNAME}/.cargo/git \
        -v ${HOME}/.cargo/registry:/home/${USERNAME}/.cargo/registry \
        -v /var/run/docker.sock:/var/run/docker.sock \
		-v $(pwd):/project \
		-w /project \
		${BUILD_IMAGE_URL}:${BUILD_IMAGE_TAG} \
        --local "${@}"
}


#############################
##### Command Utilities #####
#############################

# Remove `--release` or `--profile <profile>` from command line flags
remove-profile-flags() {
    local NEW_ARGS="${@}"
    if ( echo "${@}" | grep '\-\-release' 1>/dev/null )
    then
        NEW_ARGS="${NEW_ARGS/--release/}"
    elif ( echo "${@}" | grep '\-\-profile' 1>/dev/null )
    then
        NEW_ARGS="$(echo ${NEW_ARGS} | sed 's/--profile [^ ]\+//g')"
    fi
    echo "${NEW_ARGS}"
}

run-command() {
    local COMMAND="${1}"
    shift

    if [ ${RUNTIME_CONTEXT} = "container" ]
    then
        run-in-container "${COMMAND}" "${@}"
    elif [ ${RUNTIME_CONTEXT} = "local" ]
    then
        run-${COMMAND} "${@}"
    else
        error "Invalid value for RUNTIME_CONTEXT: ${RUNTIME_CONTEXT}"
        exit 1
    fi
}


####################
##### Commands #####
####################

run-build() {
    CHECK_TEST_ARGS="$(remove-profile-flags ${@})"

    run-check ${CHECK_TEST_ARGS}

    run-fmt-check

    run-lint ${CHECK_TEST_ARGS}

    # run-check-deps

    run-test-coverage

    info "Compiling package"
    cargo build

    run-bundle "${@}"
}

run-bundle() {
    info "Building Tauri application bundle"
    npm run tauri build "${@}"
}

run-check() {
    info "Checking Rust package for errors"
    cargo check --all-features "${@}"
    info "Type-checking frontend with svelte-check"
    npm run check
}

run-check-deps() {
    info "Checking dependencies for license compliance"
    cargo deny check licenses "${@}"

    info "Checking dependencies for security notices"
    cargo deny check advisories "${@}"

    info "Checking dependencies for trusted and banned sources"
    cargo deny check bans "${@}" && \
        cargo deny check sources "${@}"
}

run-clean() {
    info "Removing Cargo build artifacts"
    cargo clean "${@}"
}

run-dev() {
    info "Starting Tauri dev server"
    npm run tauri dev
}

run-exec() {
    info "Running command: ${*}"
    ${@}
}

run-fmt() {
    info "Formatting Rust code with Rustfmt"
    cargo fmt "${@}"
    info "Formatting frontend code with Biome"
    npm run fmt
}

run-fmt-check() {
    info "Checking Rust code format with Rustfmt"
    cargo fmt "${@}" -- --check
    info "Checking frontend code format with Biome"
    npm run fmt:check
}

run-init() {
    if ! [ -z "$(ls src/)" ]
    then
        error "Project already initialized, aborting"
        exit 1
    fi

    read -e -p "Do you want to include .gitconfig in this project's Git config [y/n]? " INCLUDE_GITCONFIG
    if ( [ "${INCLUDE_GITCONFIG,,}" = "y" ] && [ -d "./git/" ] )
    then
        git config --local include.path ../.gitconfig
    fi

    local DEFAULT_PACKAGE_NAME="$(basename $(pwd))"
    local DEFAULT_PACKAGE_TARGET="lib"
    
    read -e -p "Package name [${DEFAULT_PACKAGE_NAME}]: " PACKAGE_NAME
    PACKAGE_NAME="${PACKAGE_NAME:-${DEFAULT_PACKAGE_NAME}}"
    local PACKAGE_DIRECTORY="${PACKAGE_NAME/_/-}"
    read -e -p "Package target (bin or lib) [${DEFAULT_PACKAGE_TARGET}]: " PACKAGE_TARGET
    PACKAGE_TARGET="${PACKAGE_TARGET:-${DEFAULT_PACKAGE_TARGET}}"
    PACKAGE_TARGET="${PACKAGE_TARGET,,}"
    if ! ( [ "${PACKAGE_TARGET}" = "bin" ] || [ "${PACKAGE_TARGET}" = "lib" ] )
    then
        warn "Invalid package target '${PACKAGE_TARGET}', defaulting to ${DEFAULT_PACKAGE_TARGET}"
        PACKAGE_TARGET="${DEFAULT_PACKAGE_TARGET}"
    fi

    PACKAGE_TARGET="${PACKAGE_TARGET:-}"
    cargo new --name "${PACKAGE_NAME}" --vcs none --${PACKAGE_TARGET} "src/${PACKAGE_DIRECTORY}"

    # Update index document for documentation to point to package
    sed -i'' \
        -e "s/template_repo_rs/${PACKAGE_NAME}/g" \
        docs/index.html
}

run-lint() {
    info "Linting Rust code with Clippy"
    cargo clippy "${@}"
    info "Linting frontend code with Biome"
    npm run lint
}

run-make-docs() {
    info "Compiling package documentation"
    local DOC_BUILD_DIR=$(mktemp -d)
    cargo doc --no-deps --target-dir "${DOC_BUILD_DIR}" "${@}"
    mv ${DOC_BUILD_DIR}/doc/* docs/
    rm -rf "${DOC_BUILD_DIR}"
}

run-publish() {
    info "Creating and publishing distribution packages to crates.io"
    cargo publish "${@}"
}

run-shell() {
    info "Entering shell"
    bash
}

run-test() {
    info "Running unit and integration tests for all packages"
    cargo test --workspace --all-features "${@}"
}

run-test-coverage() {
    ## cargo llvm-cov --html --output-dir ./target/llvm-cov/hare-web-server --package hare-web-server --no-cfg-coverage
    rm -f src/**/*.profraw
}

run-update-deps() {
    info "Updating dependencies"
    cargo update "${@}"
}


################
##### Main #####
################

print-usage() {
    echo "usage: $(basename ${0}) [-h] [SUBCOMMAND]"
    echo
    echo "subcommands:"
    echo "build             compile package and build Tauri bundle (default subcommand)"
    echo "build-base        build the build container image"
    echo "bundle            build Tauri application bundle"
    echo "check             check Rust and frontend for errors"
    echo "check-deps        cargo-deny: check dependencies for license compliance, security notices, and trusted sources"
    echo "clean             cargo-clean: remove Cargo build artifacts"
    echo "dev               start Tauri dev server (frontend + backend)"
    echo "exec              execute arbitrary shell commands"
    echo "fmt               format Rust and frontend code"
    echo "init              initialize repository (should only be run once)"
    echo "lint              lint Rust and frontend code"
    echo "make-docs         cargo-doc: compile package documentation"
    echo "publish           publish package to crates.io"
    echo "push-base         push build container image to registry"
    echo "shell             start Bash shell"
    echo "test              cargo-test: run unit, documentation, and integration tests"
    echo "test-coverage     cargo-llvm-cov: run code coverage for all packages with tests"
    echo "update-deps       cargo-update: update dependencies in Cargo.lock file"
    echo
    echo "optional arguments:"
    echo "-h, --help        show this help message and exit"
    echo "-l, --local       run command on host system instead of build container"
    echo "-c, --container   run command in build container"
    echo
}


while :
do
    case "${1:-}" in
        -c|--container)
            shift
            RUNTIME_CONTEXT="container"
        ;;
        -h|--help)
            print-usage
            exit 0
        ;;
        -l|--local)
            shift
            RUNTIME_CONTEXT="local"
        ;;
        *)
            break
        ;;
    esac
done

if [ -z "${1:-}" ]
then
    COMMAND="${DEFAULT_COMMAND}"
else
    COMMAND="${1}"
    shift
fi

# These commands should explicitly run locally
if ( \
    [ "${COMMAND}" = "build-base" ] \
    || [ "${COMMAND}" = "push-base" ]
)
then
    RUNTIME_CONTEXT="local"
fi

run-command "${COMMAND}" "${@}"
