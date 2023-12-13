FROM archlinux/archlinux:base

RUN pacman -Sy --noconfirm \
    btrfs-progs \
    busybox \
    clang \
    cryptsetup \
    kmod \
    linux \
    linux-firmware \
    qemu-system-x86 \
    squashfs-tools \
    swtpm && \
    rm -rf /var/cache/pacman/pkg && \
    rm -rf /var/lib/pacman && \
    rm -rf /boot/*.img
