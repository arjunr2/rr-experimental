;; resource_drop.wat
;;
;; A minimal handwritten component that exercises resource.drop interleaved
;; with plain host calls.
;;
;; Host interface ("component:test-resources/env"):
;;   resource counter
;;   create-counter: () -> own<counter>
;;   [method]counter.increment: (borrow<counter>) -> ()
;;   ping: (u32) -> u32
;;
;; Component logic (exported "run" function):
;;   c1 = create-counter()
;;   ping(1)
;;   c2 = create-counter()
;;   c1.increment()
;;   ping(2)
;;   c1.increment()
;;   resource.drop(c1)        ;; <-- drop interleaved between pings
;;   c2.increment()
;;   ping(3)
;;   resource.drop(c2)

(component
  ;; -------------------------------------------------------------------------
  ;; Import type: component:test-resources/env
  ;; -------------------------------------------------------------------------
  (type $ty-env (;0;)
    (instance
      ;; Export the counter resource type
      (export (;0;) "counter" (type (sub resource)))

      ;; own<counter> -- used as return type for create-counter
      (type (;1;) (own 0))

      ;; create-counter: () -> own<counter>
      (type (;2;) (func (result 1)))
      (export (;1;) "create-counter" (func (type 2)))

      ;; borrow<counter> -- used as self parameter for methods
      (type (;3;) (borrow 0))

      ;; [method]counter.increment: (borrow<counter>) -> ()
      (type (;4;) (func (param "self" 3)))
      (export (;2;) "[method]counter.increment" (func (type 4)))

      ;; ping: (u32) -> u32  -- a plain host call, no resource involved
      (type (;5;) (func (param "n" u32) (result u32)))
      (export (;3;) "ping" (func (type 5)))
    )
  )
  (import "component:test-resources/env" (instance $env (;0;) (type $ty-env)))

  ;; -------------------------------------------------------------------------
  ;; Extract the counter resource type so we can call canon resource.drop
  ;; -------------------------------------------------------------------------
  (alias export $env "counter" (type $counter (;1;)))

  ;; Build core function: resource.drop counter  (i32) -> ()
  (core func $resource-drop-counter (;0;) (canon resource.drop $counter))

  ;; -------------------------------------------------------------------------
  ;; Lower imported component functions to core functions
  ;; -------------------------------------------------------------------------

  ;; create-counter: component func -> core func () -> i32
  (alias export $env "create-counter" (func $create-counter (;0;)))
  (core func $create-counter-core (;1;) (canon lower (func $create-counter)))

  ;; [method]counter.increment: component func -> core func (i32) -> ()
  (alias export $env "[method]counter.increment" (func $counter-increment (;1;)))
  (core func $counter-increment-core (;2;) (canon lower (func $counter-increment)))

  ;; ping: component func -> core func (i32) -> i32
  (alias export $env "ping" (func $ping (;2;)))
  (core func $ping-core (;3;) (canon lower (func $ping)))

  ;; -------------------------------------------------------------------------
  ;; Core module: implements the "run" function
  ;; -------------------------------------------------------------------------
  (core module $main (;0;)
    (type (;0;) (func (result i32)))           ;; create-counter: () -> i32
    (type (;1;) (func (param i32)))            ;; increment / drop: (i32) -> ()
    (type (;2;) (func (param i32) (result i32))) ;; ping: (i32) -> i32
    (type (;3;) (func))                        ;; run: () -> ()

    (import "component:test-resources/env" "create-counter"           (func $create-counter     (;0;) (type 0)))
    (import "component:test-resources/env" "[method]counter.increment" (func $counter-increment  (;1;) (type 1)))
    (import "component:test-resources/env" "ping"                      (func $ping               (;2;) (type 2)))
    (import "component:test-resources/env" "[resource-drop]counter"    (func $resource-drop      (;3;) (type 1)))

    (export "run" (func $run))

    (func $run (;4;) (type 3)
      (local $c1 i32)
      (local $c2 i32)

      ;; c1 = create-counter()
      call $create-counter
      local.set $c1

      ;; ping(1)
      i32.const 1
      call $ping
      drop

      ;; c2 = create-counter()
      call $create-counter
      local.set $c2

      ;; c1.increment()
      local.get $c1
      call $counter-increment

      ;; ping(2)
      i32.const 2
      call $ping
      drop

      ;; c1.increment() -- second increment before drop
      local.get $c1
      call $counter-increment

      ;; resource.drop(c1) -- drop c1 while c2 is still alive
      local.get $c1
      call $resource-drop

      ;; c2.increment()
      local.get $c2
      call $counter-increment

      ;; ping(3)
      i32.const 3
      call $ping
      drop

      ;; resource.drop(c2)
      local.get $c2
      call $resource-drop
    )
  )

  ;; -------------------------------------------------------------------------
  ;; Wire core module imports to the lowered functions
  ;; -------------------------------------------------------------------------
  (core instance $env-core (;0;)
    (export "create-counter"            (func $create-counter-core))
    (export "[method]counter.increment" (func $counter-increment-core))
    (export "ping"                      (func $ping-core))
    (export "[resource-drop]counter"    (func $resource-drop-counter))
  )

  (core instance $main (;1;) (instantiate $main
    (with "component:test-resources/env" (instance $env-core))
  ))

  ;; -------------------------------------------------------------------------
  ;; Lift and export "run" at the component level
  ;; -------------------------------------------------------------------------
  (alias core export $main "run" (core func $run-core (;4;)))
  (type $run-type (;2;) (func))
  (func $run-component (;3;) (type $run-type) (canon lift (core func $run-core)))
  (export "run" (func $run-component))
)
