RUST_LIBS=$(wildcard rust-componentizable/*)
RUST_TOMLS=$(addsuffix /Cargo.toml,$(RUST_LIBS))
WASIP2_SRCS=$(wildcard wasip2-rust/*.rs)
WASIP1_SRCS=$(wildcard wasip1/*.wasm)
COMPONENT_PLAIN_WATS=$(wildcard components-plain/*.wat)
COMPONENT_PLAIN_WITS=$(wildcard components-plain/*.wit)

CORE_WATS=$(wildcard core-componentizable/*.wat)
CORE_WIT_DIR=core-componentizable/wit

RUST_P1_TARGET_PATH=target/wasm32-wasip1/debug
RUST_P2_TARGET_PATH=target/wasm32-wasip2/debug

WASMTIME_DIR=../../wasmtime-rr-prototyping
P1_ADAPTER=$(WASMTIME_DIR)/target/wasm32-unknown-unknown/release/wasi_snapshot_preview1.wasm

.PHONY: all clean component-plain wasip2 wasip1 core $(RUST_TOMLS) $(RUST_LIBS)

all: core rust wasip2 wasip1 component-plain

components/wasi components/wit components:
	mkdir -p $@

.ONESHELL:
wasip2: | components/wasi
	cargo build --target=wasm32-wasip2 --manifest-path=wasip2-rust/Cargo.toml
	cp wasip2-rust/$(RUST_P2_TARGET_PATH)/*.wasm components/wasi/
	for wasm in components/wasi/*.wasm; do \
		wasm-tools print $$wasm -o $${wasm%.wasm}.wat; \
	done

.ONESHELL:
$(P1_ADAPTER):
	CARGO_PROFILE_RELEASE_LTO=fat cargo build -p wasi-preview1-component-adapter \
		--manifest-path=$(WASMTIME_DIR)/Cargo.toml \
		--target wasm32-unknown-unknown --release --no-default-features --features command

.ONESHELL:
wasip1: $(P1_ADAPTER) | components/wasi
	for wasm in $(WASIP1_SRCS); do \
		base=$$(basename $$wasm); \
		wasm-tools component new $$wasm --adapt $(P1_ADAPTER) -o components/wasi/$$base; \
		wasm-tools print components/wasi/$$base -o components/wasi/$${base%.wasm}.wat; \
	done

.ONESHELL:
core: | components components/wit
	@for core in $(CORE_WATS); do \
		base=$$(basename $${core%%.*}); \
		echo "Processing '$$core'"; \
		set -x; \
		wasm-tools component embed $(CORE_WIT_DIR)/$$base.wit $$core -t -o tmp.wat && \
		wasm-tools component new tmp.wat -t -o components/$$base.wat && \
		wasm-tools parse components/$$base.wat -o components/$$base.wasm && \
		wasm-tools component wit components/$$base.wat -o components/wit/$$base.wit; \
		set +x; \
		rm -f tmp.wat; \
	done

.ONESHELL:
rust: $(RUST_LIBS)

.ONESHELL:
$(RUST_LIBS): | components components/wit
	cargo component build --manifest-path=$@/Cargo.toml
	set -x
	wasm-tools parse -t $@/$(RUST_P1_TARGET_PATH)/`basename $@`.wasm -o components/`basename $@`.wat
	cp $@/$(RUST_P1_TARGET_PATH)/`basename $@`.wasm components/`basename $@`.wasm
	cp $@/wit/*.wit components/wit/`basename $@`.wit

component-plain: | components components/wit
	for wat in $(COMPONENT_PLAIN_WATS); do \
		base=$$(basename $${wat%.*}); \
		wasm-tools parse $$wat -o components/$$base.wasm; \
		cp $$wat components/$$base.wat; \
	done
	for wit in $(COMPONENT_PLAIN_WITS); do \
		cp $$wit components/wit/; \
	done

clean:
	rm -r components
