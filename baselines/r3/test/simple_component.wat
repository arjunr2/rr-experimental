(component
  ;; A simple self-contained component with no host imports.
  ;; The core module just does some memory operations.
  (core module $m
    (memory (export "memory") 1)

    (func (export "_start")
      ;; Store some values in memory
      (i32.store (i32.const 0) (i32.const 42))
      (i32.store (i32.const 4) (i32.const 100))
      ;; Load and add
      (i32.store (i32.const 8)
        (i32.add
          (i32.load (i32.const 0))
          (i32.load (i32.const 4))
        )
      )
    )
  )

  (core instance $i (instantiate $m))
  (alias core export $i "_start" (core func $start))
  (type (func))
  (func $start (canon lift (core func $start)))
  (export "run" (func $start))
)
