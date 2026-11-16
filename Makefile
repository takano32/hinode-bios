.PHONY: all build run test clean

all: build

build:
	cargo build --release
	objcopy -O binary target/aarch64-unknown-none/release/hinode_bios target/aarch64-unknown-none/release/hinode_bios.bin

run: build
	qemu-system-aarch64 -M virt -cpu cortex-a57 -display none -serial stdio -bios target/aarch64-unknown-none/release/hinode_bios.bin

test: build
	bash tests/run_qemu_tests.sh

clean:
	cargo clean
