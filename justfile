#!/usr/bin/env just --justfile

VERSION := `cargo pkgid | sed 's/.*#//'`

linux:
    cargo build --release
    rm -rf output/linux
    mkdir -p output/linux/llagram
    cp target/release/llagram output/linux/llagram/
    cp llagram.ron output/linux/llagram/
    cd output/linux && tar cJvf llagram_{{VERSION}}.linux.x86-64.tar.xz llagram && mv llagram_{{VERSION}}.linux.x86-64.tar.xz ..

linux-arm64:
    cargo build --release --target=aarch64-unknown-linux-gnu
    rm -rf output/linux-arm64
    mkdir -p output/linux-arm64/llagram
    cp target/aarch64-unknown-linux-gnu/release/llagram output/linux-arm64/llagram/
    cp llagram.ron output/linux-arm64/llagram/
    cd output/linux-arm64 && tar cJvf llagram_{{VERSION}}.linux.arm64.tar.xz llagram && mv llagram_{{VERSION}}.linux.arm64.tar.xz ..


release: linux linux-arm64
