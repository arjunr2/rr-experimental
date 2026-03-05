(module
  ;; Import fd_write from WASI
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))

  ;; Memory
  (memory (export "memory") 1)

  ;; Exported function: writes "Hello\n" to stdout
  (func (export "_start")
    ;; Set up the iovec at address 0
    ;; iovec.buf = 100 (pointer to string data)
    (i32.store (i32.const 0) (i32.const 100))
    ;; iovec.buf_len = 6
    (i32.store (i32.const 4) (i32.const 6))

    ;; Store "Hello\n" at address 100
    (i32.store (i32.const 100) (i32.const 0x6c6c6548))  ;; "Hell"
    (i32.store16 (i32.const 104) (i32.const 0x0a6f))    ;; "o\n"

    ;; Call fd_write(stdout=1, iovs=0, iovs_len=1, nwritten=200)
    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 200)))
  )
)
