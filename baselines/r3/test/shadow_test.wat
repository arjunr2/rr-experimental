(module
  (memory (export "memory") 1)
  (func (export "_start")
    ;; Store 42 at address 0
    (i32.store (i32.const 0) (i32.const 42))
    ;; Load it back (should match shadow → no nop)
    (drop (i32.load (i32.const 0)))
    ;; Store 100 at address 4
    (i32.store (i32.const 4) (i32.const 100))
    ;; Load from address 4 (should match → no nop)
    (drop (i32.load (i32.const 4)))
  )
)
