RUST_LIBS=$(wildcard rust-componentizable/*)
RUST_TOMLS=$(addsuffix /Cargo.toml,$(RUST_LIBS))
WASI_SRCS=$(wildcard wasi/*.rs)
COMPONENT_PLAIN_WATS=$(wildcard components-plain/*.wat)
COMPONENT_PLAIN_WITS=$(wildcard components-plain/*.wit)

RUST_P1_TARGET_PATH=target/wasm32-wasip1/debug
RUST_P2_TARGET_PATH=target/wasm32-wasip2/debug

.PHONY: all clean component-plain wasi $(RUST_TOMLS) $(RUST_LIBS)

all: core rust wasi component-plain

.ONESHELL:
wasi:
	cargo build --target=wasm32-wasip2 --manifest-path=wasi/Cargo.toml
	mkdir -p components/wasi
	cp wasi/$(RUST_P2_TARGET_PATH)/*.wasm components/wasi/
	for wasm in components/wasi/*.wasm; do \
		wasm-tools print $$wasm -o $${wasm%.wasm}.wat; \
	done

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

component-plain:
	mkdir -p components
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
