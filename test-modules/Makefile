RUST_LIBS=$(wildcard rust-componentizable/*)
RUST_TOMLS=$(addsuffix /Cargo.toml,$(RUST_LIBS))
WASI_SRCS=$(wildcard wasi/*.rs)

RUST_P1_TARGET_PATH=target/wasm32-wasip1/debug
RUST_P2_TARGET_PATH=target/wasm32-wasip2/debug

.PHONY: all clean wasi $(RUST_TOMLS) $(RUST_LIBS)

all: core rust wasi

.ONESHELL:
wasi:
	cargo build --target=wasm32-wasip2 --manifest-path=wasi/Cargo.toml
	mkdir -p components/wasi
	cp wasi/$(RUST_P2_TARGET_PATH)/*.wasm components/wasi/

core:
	./core-componentize.sh

.ONESHELL:
rust: $(RUST_LIBS)

.ONESHELL:
$(RUST_LIBS):
	cargo component build --manifest-path=$@/Cargo.toml
	set -x
	wasm-tools parse -t $@/$(RUST_P1_TARGET_PATH)/`basename $@`.wasm -o components/`basename $@`.wat
	cp $@/$(RUST_P1_TARGET_PATH)/`basename $@`.wasm components/`basename $@`.wasm
	cp $@/wit/*.wit components/wit/`basename $@`.wit

clean:
	rm -r components
