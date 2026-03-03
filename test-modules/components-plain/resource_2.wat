(component
  (type $r (resource (rep i32)))
  (core func $rep (canon resource.rep $r))
  (core func $new (canon resource.new $r))
  (core func $drop (canon resource.drop $r))

  (core module $m
    (import "" "rep" (func $rep (param i32) (result i32)))
    (import "" "new" (func $new (param i32) (result i32)))
    (import "" "drop" (func $drop (param i32)))

    (func $start
      (local $r1 i32)
      (local $r2 i32)
      (local $r3 i32)
      (local $r4 i32)

      ;; resources assigned sequentially
      (local.set $r1 (call $new (i32.const 100)))
      (if (i32.ne (local.get $r1) (i32.const 1)) (then (unreachable)))

      (local.set $r2 (call $new (i32.const 200)))
      (if (i32.ne (local.get $r2) (i32.const 2)) (then (unreachable)))

      (local.set $r3 (call $new (i32.const 300)))
      (if (i32.ne (local.get $r3) (i32.const 3)) (then (unreachable)))

      ;; representations all look good
      (if (i32.ne (call $rep (local.get $r1)) (i32.const 100)) (then (unreachable)))
      (if (i32.ne (call $rep (local.get $r2)) (i32.const 200)) (then (unreachable)))
      (if (i32.ne (call $rep (local.get $r3)) (i32.const 300)) (then (unreachable)))

      ;; reallocate r2
      (call $drop (local.get $r2))
      (local.set $r2 (call $new (i32.const 400)))

      ;; should have reused index 1
      (if (i32.ne (local.get $r2) (i32.const 2)) (then (unreachable)))

      ;; representations all look good
      (if (i32.ne (call $rep (local.get $r1)) (i32.const 100)) (then (unreachable)))
      (if (i32.ne (call $rep (local.get $r2)) (i32.const 400)) (then (unreachable)))
      (if (i32.ne (call $rep (local.get $r3)) (i32.const 300)) (then (unreachable)))

      ;; deallocate, then reallocate
      (call $drop (local.get $r1))
      (call $drop (local.get $r2))
      (call $drop (local.get $r3))

      (local.set $r1 (call $new (i32.const 500)))
      (local.set $r2 (call $new (i32.const 600)))
      (local.set $r3 (call $new (i32.const 700)))

      ;; representations all look good
      (if (i32.ne (call $rep (local.get $r1)) (i32.const 500)) (then (unreachable)))
      (if (i32.ne (call $rep (local.get $r2)) (i32.const 600)) (then (unreachable)))
      (if (i32.ne (call $rep (local.get $r3)) (i32.const 700)) (then (unreachable)))

      ;; indices should be lifo
      (if (i32.ne (local.get $r1) (i32.const 3)) (then (unreachable)))
      (if (i32.ne (local.get $r2) (i32.const 2)) (then (unreachable)))
      (if (i32.ne (local.get $r3) (i32.const 1)) (then (unreachable)))

      ;; bump one more time
      (local.set $r4 (call $new (i32.const 800)))
      (if (i32.ne (local.get $r4) (i32.const 4)) (then (unreachable)))

      ;; deallocate everything
      (call $drop (local.get $r1))
      (call $drop (local.get $r2))
      (call $drop (local.get $r3))
      (call $drop (local.get $r4))
    )

    (start $start)
  )
  (core instance (instantiate $m
    (with "" (instance
      (export "rep" (func $rep))
      (export "new" (func $new))
      (export "drop" (func $drop))
    ))
  ))
)
