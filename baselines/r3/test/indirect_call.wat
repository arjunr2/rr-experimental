(module
  (type $t0 (func (param i32) (result i32)))
  (import "env" "double" (func $double (type $t0)))
  (import "env" "triple" (func $triple (type $t0)))
  (table 4 funcref)
  (memory (export "memory") 1)
  ;; Element segment populating the table:
  ;; table[0] = $double (import 0)
  ;; table[1] = $triple (import 1)
  ;; table[2] = $local_add1
  (elem (i32.const 0) func $double $triple $local_add1)
  (func $local_add1 (type $t0) (param i32) (result i32)
    local.get 0
    i32.const 1
    i32.add
  )
  (func (export "_start")
    ;; Direct call to import
    i32.const 5
    call $double
    ;; Store result
    i32.const 0
    i32.store

    ;; Indirect call through table[0] -> should go through trampoline for $double
    i32.const 10
    i32.const 0
    call_indirect (type $t0)
    i32.const 4
    i32.store

    ;; Indirect call through table[1] -> should go through trampoline for $triple
    i32.const 10
    i32.const 1
    call_indirect (type $t0)
    i32.const 8
    i32.store

    ;; Indirect call through table[2] -> local func, no trampoline
    i32.const 10
    i32.const 2
    call_indirect (type $t0)
    i32.const 12
    i32.store
  )
)
