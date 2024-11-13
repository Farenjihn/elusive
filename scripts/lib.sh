TEST_DIR="$(git rev-parse --show-toplevel)/boot"

MAPPER="elusiveboot"
UUID="f8db7e4d-84a1-40d7-acfa-6cbc20ac9960"

SWTPM_PID=0

mount_path() {
    echo -n "${TEST_DIR}/mountpoint"
}

image_path() {
    echo -n "${TEST_DIR}/image/debian-$1.img"
}

cache_path() {
    echo -n "${TEST_DIR}/image/debian-$1/cache"
}

kernel_path() {
    echo -n "${TEST_DIR}/image/debian-$1/kernel.bin"
}

initramfs_path() {
    echo -n "${TEST_DIR}/image/initramfs.bin"
}

swtpm_state_path() {
    echo -n "${TEST_DIR}/swtpm.state"
}

swtpm_socket_path() {
    echo -n "${TEST_DIR}/swtpm.sock"
}

setup_image() {
    local RELEASE="$1"

    local MOUNT_PATH="$(mount_path)"
    local IMAGE_PATH="$(image_path ${RELEASE})"
    local CACHE_PATH="$(cache_path ${RELEASE})"

    mkdir -p $(dirname "${IMAGE_PATH}")
    mkdir -p "${CACHE_PATH}"
    mkdir -p "${MOUNT_PATH}"

    if [[ ! -f "${IMAGE_PATH}" ]]; then
        # create file
        echo "[+] Creating image file"
        truncate -s 2G "${IMAGE_PATH}"

        # setup header
        echo "[+] Setting up LUKS"
        echo -n "123" | sudo cryptsetup luksFormat --uuid "${UUID}" -d - "${IMAGE_PATH}"
        echo -n "123" | sudo cryptsetup luksOpen -d - "${IMAGE_PATH}" "${MAPPER}"

        # create and mount filesystem
        echo "[+] Formatting image"
        sudo mkfs.ext4 "/dev/mapper/${MAPPER}"
        sudo mount "/dev/mapper/${MAPPER}" "${MOUNT_PATH}"

        # install debian
        echo "[+] Installing debian ${RELEASE}"
        sudo debootstrap \
            --cache-dir "${CACHE_PATH}" \
            --include "linux-image-amd64 libtss2-dev" \
            "${RELEASE}" "${MOUNT_PATH}"

        sudo chroot boot/mountpoint/ /bin/bash -c 'echo root:root | chpasswd'
    else
        # if image exists, simply use it
        echo "[+] Reusing existing image"
        echo -n "123" | sudo cryptsetup luksOpen -d - "${IMAGE_PATH}" "${MAPPER}"
        sudo mount "/dev/mapper/${MAPPER}" "${MOUNT_PATH}"
    fi

    local KERNEL_PATH="$(kernel_path ${RELEASE})"
    local INITRAMFS_PATH="$(initramfs_path ${RELEASE})"

    # copy kernel
    cp "${MOUNT_PATH}/boot/vmlinuz"-* "${KERNEL_PATH}"

    # generate initramfs
    cargo run -- initramfs \
        --skip-default-paths \
        --config contrib/config/elusive.yaml \
        --confdir contrib/config/elusive.d \
        --modules "${MOUNT_PATH}/lib/modules/"*-amd64 \
        --encoder zstd \
        --output "${INITRAMFS_PATH}"

    # unmount image
    echo "[+] Unmounting image"
    sudo umount "${MOUNT_PATH}"
    sudo cryptsetup luksClose "${MAPPER}"
}

start_tpm() {
    swtpm socket \
        --tpmstate "backend-uri=file:///$(swtpm_state_path)" \
        --ctrl "type=unixio,path=$(swtpm_socket_path)" \
        --tpm2 &
}

boot_image() {
    local RELEASE="$1"
    local IMAGE_PATH="${TEST_DIR}/image/debian-${RELEASE}.img"
    local KERNEL_PATH="$(kernel_path ${RELEASE})"
    local INITRAMFS_PATH="$(initramfs_path ${RELEASE})"

    MACHINE="q35"
    [ -c /dev/kvm ] && MACHINE="${MACHINE},accel=kvm"

    set +e
    echo "[+] Booting image"
    start_tpm
    qemu-system-x86_64 \
        -machine "${MACHINE}" \
        -m "2048" \
        -display "gtk" \
        -vga "std" \
        -drive "file=${IMAGE_PATH},if=virtio,format=raw" \
        -chardev "socket,id=chrtpm,path=$(swtpm_socket_path)" \
        -tpmdev "emulator,id=tpm0,chardev=chrtpm" \
        -device "tpm-tis,tpmdev=tpm0" \
        -object "rng-random,id=rng0,filename=/dev/urandom"\
        -device "virtio-rng-pci,rng=rng0" \
        -kernel "${KERNEL_PATH}" \
        -initrd "${INITRAMFS_PATH}" \
        -append "root=/dev/mapper/root rd.luks.name=${UUID}=root"
}
