#!/bin/bash

set -e

# export RUST_LOG=debug

ELUSIVE="${PWD}/target/debug/elusive"
OUTPUT_DIR="${PWD}/.output"

ensure() {
    if [ ! -f "$1" ]; then
        echo "$2"
        exit 1
    fi
}

elusive() {
    $ELUSIVE "$@"
}

qemu() {
    swtpm socket \
        --tpmstate=backend-uri=file://swtpm.state \
        --ctrl type=unixio,path=swtpm.sock \
        --tpm2 &

    MACHINE="pc-q35-5.0"
    [ -c /dev/kvm ] && MACHINE="${MACHINE},accel=kvm"

    set +e
    qemu-system-x86_64 \
        -machine "${MACHINE}" \
        -m 512 \
        -no-reboot \
        -nographic \
        -chardev socket,id=chrtpm,path=swtpm.sock \
        -tpmdev emulator,id=tpm0,chardev=chrtpm \
        -device tpm-tis,tpmdev=tpm0 \
        -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
        -drive file="${OUTPUT_DIR}/root.squashfs",if=virtio,format=raw \
        -kernel vmlinuz \
        -initrd "$1" \
        -append "console=ttyS0,115200 panic=-1 root=/dev/vda rootfstype=squashfs systemd.default_timeout_start_sec=15"

    ret=$?
    set -e

    [ -f swtpm.state ] && rm swtpm.state
    [ -e swtpm.sock ] && rm swtpm.sock

    if [ ${ret} -ne 85 ]; then
        echo "QEMU exited with code=${ret}"
        exit 1
    fi
}

cleanup() {
    rm -rf "${OUTPUT_DIR}"
}
trap cleanup EXIT

# check dependencies
NAG="you need to run 'hack/build.sh' first"
ensure "${ELUSIVE}" "${NAG}"
ensure \
    shim/target/x86_64-unknown-linux-musl/debug/shim \
    "${NAG}"

ensure "vmlinuz" "copy a kernel image to vmlinuz and ensure modules are properly installed"

# get the kernel release from the vmlinuz file
KERNEL_RELEASE="$(file -bL vmlinuz | sed 's/.*version //;s/ .*//')"
MODULES_DIR="/lib/modules/${KERNEL_RELEASE}"

# create output directory
mkdir -p "${OUTPUT_DIR}"

# build microcode archive
pushd samples > /dev/null

elusive microcode \
    --skip-default-paths \
    --config config/ucode.yaml \
    --output "${OUTPUT_DIR}/microcode"

# build both initramfs archives
elusive initramfs \
    --skip-default-paths \
    --config config/basic.yaml \
    --modules "${MODULES_DIR}" \
    --ucode "${OUTPUT_DIR}/microcode" \
    --output "${OUTPUT_DIR}/initramfs.basic"

export LD_LIBRARY_PATH="/lib/systemd"
elusive initramfs \
    --skip-default-paths \
    --config config/systemd.yaml \
    --confdir config/systemd.d \
    --modules "${MODULES_DIR}" \
    --ucode "${OUTPUT_DIR}/microcode" \
    --output "${OUTPUT_DIR}/initramfs.systemd"

popd > /dev/null

# create rootfs
ROOTFS="${OUTPUT_DIR}/rootfs"
SQUASHFS="${OUTPUT_DIR}/root.squashfs"

mkdir -p "${ROOTFS}"/{dev,etc,proc,run,sbin,sys}
touch "${ROOTFS}/etc/os-release"

cp shim/target/x86_64-unknown-linux-musl/debug/shim \
    "${ROOTFS}/sbin/init"

[ -f "${SQUASHFS}" ] && rm "${SQUASHFS}"
mksquashfs "${ROOTFS}" "${SQUASHFS}"

# run qemu
qemu "${OUTPUT_DIR}/initramfs.basic"
qemu "${OUTPUT_DIR}/initramfs.systemd"
