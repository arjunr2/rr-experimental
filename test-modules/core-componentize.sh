#!/bin/bash

# Generates components from all componentizable-core modules

meta=tmp.wat

mkdir -p components/wit

core_wit_dir=core-componentizable/wit

for core in core-componentizable/*.wat; do
    base=$(basename ${core%%.*})
    base_wit=$core_wit_dir/$base.wit
    component=components/$base.wat
    component_wasm=components/$base.wasm
    new_wit=components/wit/$base.wit
    echo "Processing '$core' with '$base_wit'"
    set -x
    wasm-tools component embed $base_wit $core -t -o $meta
    wasm-tools component new $meta -t -o $component
    wasm-tools parse $component -o $component_wasm
    wasm-tools component wit $component -o $new_wit
    set +x
    rm $meta
done
