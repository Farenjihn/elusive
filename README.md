# Elusive

Elusive is a simple Rust program that helps in generating initramfs archives by reading a declarative configuration.

## Features

Elusive can help you generate an initramfs archive that includes exactly what you want. More specifically it can:

- Create a compressed cpio archive from a declarative configuration file written in TOML.
- Create a compressed microcode bundle archive for early CPU microcode loading by the Linux kernel, which can be included in your initramfs.

However, this project does not manage what happens in your initramfs once your system boots. Its sole purpose is to create the archive. Writing (or adding) the init program, managing hooks, events, or actually ensuring that the resulting initramfs will allow you to boot your system is the user's responsibility.

## How

A sample configuration file is available in the repository. Using the CLI should be straightforward enough, but here is an example:

```sh
elusive microcode --output ucode.img
elusive initramfs --ucode ucode.img --output initramfs.gz
```

By default, configuration is read from `/etc/elusive.toml`, but the path can be selected through the `--config` command-line argument at runtime.

## Why

I wrote this in my free time to help me customize a Gentoo system I use in attack-defense CTFs, and also for fun. In my initial use case, I wanted to have control over the entire boot process for my box through the use of secure boot and various hardware components like TPMs.

For that reason, there is only basic support to add kernel modules in the initramfs. Since the original use case means having control over the kernel configuration, the assumption is users can configure their kernel and select the needed features as built-in, or be knowledgable enough to load them in the initramfs.

This is probably not useful if you do not want to write your own init script, or do something that is not supported by the likes of `mkinicpio`, `dracut` or other "batteries-included" initramfs generators.

## Testing

A simple script using `qemu` is included in the repository for quick testing. For now it hardcodes the path to the kernel on Arch Linux but this can be changed if necessary. To use it simply run:

```sh
./qemu.sh
```

Once in the VM, you can shut it down using `poweroff -f`.
