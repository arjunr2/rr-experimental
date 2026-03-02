;; resource_guest_dtor.wat
;;
;; A component that defines a resource with a guest-side destructor.
;; The dtor increments a counter in memory each time it's called.
;; The exported "run" function creates resources, drops them, and
;; verifies the dtor was called the expected number of times.

(component
  ;; -------------------------------------------------------------------
  ;; Module that provides the destructor and shared memory
  ;; -------------------------------------------------------------------
  (core module $dtor_mod
    (memory (export "mem") 1)

    ;; Destructor: increments a counter at mem[0] each time called.
    ;; Param is the rep (i32).
    (func (export "dtor") (param $rep i32)
      (i32.store (i32.const 0)
        (i32.add (i32.load (i32.const 0)) (i32.const 1)))
    )
  )
  (core instance $dtor_inst (instantiate $dtor_mod))

  ;; -------------------------------------------------------------------
  ;; Define resource type with the guest dtor
  ;; -------------------------------------------------------------------
  (type $r (resource (rep i32) (dtor (func $dtor_inst "dtor"))))

  ;; Canon operations on $r
  (core func $new (canon resource.new $r))
  (core func $drop (canon resource.drop $r))
  (core func $rep (canon resource.rep $r))

  ;; -------------------------------------------------------------------
  ;; Main module that exercises the resource
  ;; -------------------------------------------------------------------
  (core module $main
    (import "" "new" (func $new (param i32) (result i32)))
    (import "" "drop" (func $drop (param i32)))
    (import "" "rep" (func $rep (param i32) (result i32)))
    (import "" "mem" (memory 1))

    (func (export "run")
      (local $r1 i32)
      (local $r2 i32)
      (local $r3 i32)

      ;; Create three resources with different reps
      (local.set $r1 (call $new (i32.const 10)))
      (local.set $r2 (call $new (i32.const 20)))
      (local.set $r3 (call $new (i32.const 30)))

      ;; Verify handles and reps
      (if (i32.ne (call $rep (local.get $r1)) (i32.const 10)) (then (unreachable)))
      (if (i32.ne (call $rep (local.get $r2)) (i32.const 20)) (then (unreachable)))
      (if (i32.ne (call $rep (local.get $r3)) (i32.const 30)) (then (unreachable)))

      ;; Drop r1 -- dtor should fire, counter becomes 1
      (call $drop (local.get $r1))
      (if (i32.ne (i32.load (i32.const 0)) (i32.const 1)) (then (unreachable)))

      ;; Drop r3 -- dtor should fire, counter becomes 2
      (call $drop (local.get $r3))
      (if (i32.ne (i32.load (i32.const 0)) (i32.const 2)) (then (unreachable)))

      ;; Drop r2 -- dtor should fire, counter becomes 3
      (call $drop (local.get $r2))
      (if (i32.ne (i32.load (i32.const 0)) (i32.const 3)) (then (unreachable)))
    )
  )

  (core instance $main_inst (instantiate $main
    (with "" (instance
      (export "new" (func $new))
      (export "drop" (func $drop))
      (export "rep" (func $rep))
      (export "mem" (memory $dtor_inst "mem"))
    ))
  ))

  ;; -------------------------------------------------------------------
  ;; Lift and export "run"
  ;; -------------------------------------------------------------------
  (alias core export $main_inst "run" (core func $run_core))
  (type $run_ty (func))
  (func (export "run") (type $run_ty) (canon lift (core func $run_core)))
)
