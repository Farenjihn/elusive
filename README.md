# Elusive

Elusive is a simple Rust program that helps in generating initramfs archives by reading a declarative configuration.

## Features

Elusive can help you generate an initramfs archive that includes exactly what you want. More specifically it can:

- Create a compressed cpio archive from a declarative configuration file written in TOML.
- Create a compressed microcode bundle archive for early CPU microcode loading by the Linux kernel, which can be included in your initramfs.

However, this project does not manage what happens in your initramfs once your system boots. Its sole purpose is to create the archive. Writing (or adding) the init program, managing hooks, events, or actually ensuring that the resulting initramfs will allow you to boot your system is your responsibility.

## How

A sample configuration file is available in the repository. Using the CLI should be straightforward enough, but here is an example:

```sh
elusive microcode -o - | elusive initramfs --ucode - -o initramfs.gz
```

By default, configuration is read from `/etc/elusive/config.toml`, but the path can be selected through a command-line argument at runtime.

## Why

I wrote this in my free time to help me customize a Gentoo system I use in attack-defense CTFs, and also for fun. In my initial use case, I wanted to have control over the entire boot process for my box through the use of secure boot and various hardware components like TPMs.

This is probably not useful if you do not want to write your own init script, or do something that is not supported by the likes of `mkinicpio`, `dracut` or other "batteries-included" initramfs generators.
