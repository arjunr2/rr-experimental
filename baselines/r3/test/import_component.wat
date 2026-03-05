(component
  ;; Component that imports WASI random to test import replay.
  ;; The core module imports a "env.get_random" function and stores its results.
  (core module $m
    (import "env" "get_random" (func $get_random (result i32)))
    (memory (export "memory") 1)

    (func (export "_start")
      ;; Get two random values and store them
      (i32.store (i32.const 0) (call $get_random))
      (i32.store (i32.const 4) (call $get_random))
      ;; Sum them
      (i32.store (i32.const 8)
        (i32.add
          (i32.load (i32.const 0))
          (i32.load (i32.const 4))
        )
      )
    )
  )

  ;; Provide the import via a component-level function
  (import "get-random" (func $get_random (result s32)))
  (core func $get_random_lower (canon lower (func $get_random)))
  (core instance $env (export "get_random" (func $get_random_lower)))

  (core instance $i (instantiate $m (with "env" (instance $env))))
  (alias core export $i "_start" (core func $start))
  (type (func))
  (func $start (canon lift (core func $start)))
  (export "run" (func $start))
)
