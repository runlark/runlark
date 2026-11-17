(module
  (import "$root" "log" (func $host_log (param i32 i32 i32 i32)))
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 4096))

  (data (i32.const 0x0400) "info")
  (data (i32.const 0x0410) "validating run")
  (data (i32.const 0x0420) "all validation checks passed")
  (data (i32.const 0x0440) "validation failed")
  (data (i32.const 0x0460) "no changed files recorded")
  (data (i32.const 0x0480) "empty diff (0 bytes)")
  (data (i32.const 0x04a0) "no check evidence provided")
  (data (i32.const 0x04c0) "one or more checks failed")
  (data (i32.const 0x04e0) "task-id and run-id must be non-empty")

  (func (export "cabi_realloc")
    (param $old_ptr i32) (param $old_size i32) (param $align i32) (param $new_size i32)
    (result i32)
    (local $ptr i32)
    (local $mask i32)
    (local $end i32)
    (local $pages i32)
    (if (i32.eqz (local.get $new_size))
      (then (return (local.get $align)))
    )
    (local.set $mask (i32.sub (local.get $align) (i32.const 1)))
    (local.set $ptr
      (i32.and
        (i32.add (global.get $heap) (local.get $mask))
        (i32.xor (local.get $mask) (i32.const -1))
      )
    )
    (local.set $end (i32.add (local.get $ptr) (local.get $new_size)))
    (local.set $pages (i32.shl (memory.size) (i32.const 16)))
    (if (i32.gt_u (local.get $end) (local.get $pages))
      (then
        (if
          (i32.lt_s
            (memory.grow
              (i32.shr_u
                (i32.add (i32.sub (local.get $end) (local.get $pages)) (i32.const 65535))
                (i32.const 16)
              )
            )
            (i32.const 0)
          )
          (then (unreachable))
        )
      )
    )
    (global.set $heap (local.get $end))
    (local.get $ptr)
  )

  (func $push_finding (param $count i32) (param $ptr i32) (param $len i32) (result i32)
    (local $slot i32)
    (local.set $slot (i32.add (i32.const 0x0200) (i32.mul (local.get $count) (i32.const 8))))
    (i32.store (local.get $slot) (local.get $ptr))
    (i32.store (i32.add (local.get $slot) (i32.const 4)) (local.get $len))
    (i32.add (local.get $count) (i32.const 1))
  )

  (func (export "runlark:workflow/validator@0.1.0#validate-run")
    (param $task_ptr i32) (param $task_len i32)
    (param $run_ptr i32) (param $run_len i32)
    (param $stage i32)
    (param $changed_ptr i32) (param $changed_len i32)
    (param $checks_ptr i32) (param $checks_len i32)
    (param $diff_bytes i64)
    (param $repair_attempt i32)
    (result i32)
    (local $findings i32)
    (local $idx i32)
    (local $entry i32)
    (local $any_failed i32)

    (if
      (i32.or
        (i32.eqz (local.get $task_len))
        (i32.eqz (local.get $run_len))
      )
      (then
        (i32.store (i32.const 0x0100) (i32.const 1))
        (i32.store (i32.const 0x0104) (i32.const 0x04e0))
        (i32.store (i32.const 0x0108) (i32.const 36))
        (return (i32.const 0x0100))
      )
    )

    (call $host_log (i32.const 0x0400) (i32.const 4) (i32.const 0x0410) (i32.const 14))

    (local.set $findings (i32.const 0))

    (if (i32.eqz (local.get $changed_len))
      (then
        (local.set $findings
          (call $push_finding (local.get $findings) (i32.const 0x0460) (i32.const 25))
        )
      )
    )

    (if (i64.eqz (local.get $diff_bytes))
      (then
        (local.set $findings
          (call $push_finding (local.get $findings) (i32.const 0x0480) (i32.const 20))
        )
      )
    )

    (if (i32.eqz (local.get $checks_len))
      (then
        (local.set $findings
          (call $push_finding (local.get $findings) (i32.const 0x04a0) (i32.const 26))
        )
      )
      (else
        (local.set $idx (i32.const 0))
        (local.set $any_failed (i32.const 0))
        (block $done
          (loop $next
            (br_if $done (i32.ge_u (local.get $idx) (local.get $checks_len)))
            (local.set $entry
              (i32.add (local.get $checks_ptr) (i32.mul (local.get $idx) (i32.const 16)))
            )
            (if
              (i32.or
                (i32.eqz (i32.load8_u (i32.add (local.get $entry) (i32.const 8))))
                (i32.ne (i32.load (i32.add (local.get $entry) (i32.const 12))) (i32.const 0))
              )
              (then (local.set $any_failed (i32.const 1)))
            )
            (local.set $idx (i32.add (local.get $idx) (i32.const 1)))
            (br $next)
          )
        )
        (if (local.get $any_failed)
          (then
            (local.set $findings
              (call $push_finding (local.get $findings) (i32.const 0x04c0) (i32.const 25))
            )
          )
        )
      )
    )

    (i32.store (i32.const 0x0100) (i32.const 0))
    (if (i32.eqz (local.get $findings))
      (then
        (i32.store (i32.const 0x0104) (i32.const 0))
        (i32.store (i32.const 0x0108) (i32.const 0x0420))
        (i32.store (i32.const 0x010c) (i32.const 28))
        (i32.store (i32.const 0x0110) (i32.const 0))
        (i32.store (i32.const 0x0114) (i32.const 0))
      )
      (else
        (i32.store (i32.const 0x0104) (i32.const 1))
        (i32.store (i32.const 0x0108) (i32.const 0x0440))
        (i32.store (i32.const 0x010c) (i32.const 17))
        (i32.store (i32.const 0x0110) (i32.const 0x0200))
        (i32.store (i32.const 0x0114) (local.get $findings))
      )
    )
    (i32.const 0x0100)
  )
)
