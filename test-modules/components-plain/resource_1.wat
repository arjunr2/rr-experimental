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
      (local $r i32)
      (local.set $r (call $new (i32.const 100)))

      (if (i32.ne (local.get $r) (i32.const 1)) (then (unreachable)))
      (if (i32.ne (call $rep (local.get $r)) (i32.const 100)) (then (unreachable)))

      (call $drop (local.get $r))
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
