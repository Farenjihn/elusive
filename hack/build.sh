#!/bin/bash

set -e

# build elusive
cargo build

# build qemu shim
pushd shim > /dev/null
cargo build
popd > /dev/null
