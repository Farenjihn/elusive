#!/bin/bash

set -e

SCRIPT_DIR="$(dirname -- ${BASH_SOURCE[0]})"
RELEASE="bookworm"

source "${SCRIPT_DIR}/lib.sh"

setup_image "${RELEASE}"
boot_image "${RELEASE}"
