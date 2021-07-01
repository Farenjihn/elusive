#!/bin/bash

set -e

TMP=$(mktemp -d /tmp/elusive.XXXXXXXXXX) || exit 1

cargo build

# export RUST_LOG=debug

# build microcode archive
target/debug/elusive microcode \
    --config example.toml \
    --encoder zstd \
    --output "${TMP}/ucode"

# build initramfs
target/debug/elusive initramfs \
    --config example.toml \
    --encoder zstd \
    --ucode "${TMP}/ucode" \
    --output "${TMP}/initramfs"

# start a VM using KVM that directly
# boots a kernel and the generated
# initramfs
qemu-system-x86_64 \
    -machine pc-q35-5.0,accel=kvm \
    -cpu host \
    -m 512 \
    -nographic \
    -kernel /boot/vmlinuz-linux \
    -initrd "${TMP}/initramfs" \
    -append "console=ttyS0,115200"

rm -rf "${TMP}"
