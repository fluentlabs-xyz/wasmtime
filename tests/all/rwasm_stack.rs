//! The rwasm stack-limit emulation (`Config::rwasm_stack_limits`): the depth counter a call
//! site publishes and the counters a tail call has to publish again.

#![cfg(not(miri))]

use wasmtime::*;

const MAX_CALL_DEPTH: u32 = 1024;
const MAX_STACK_SLOTS: u32 = 8192 + 13;

/// Appends the `rwasm.frames` custom section the engine requires: one little-endian `u32`
/// `StackCheck` height per function of the module, imports first.
fn with_frame_heights(wasm: &[u8], heights: &[u32]) -> Vec<u8> {
    fn leb(out: &mut Vec<u8>, mut value: u32) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }
    let mut payload = Vec::new();
    leb(&mut payload, RWASM_FRAMES_SECTION.len() as u32);
    payload.extend_from_slice(RWASM_FRAMES_SECTION.as_bytes());
    for height in heights {
        payload.extend_from_slice(&height.to_le_bytes());
    }
    let mut out = wasm.to_vec();
    out.push(0);
    leb(&mut out, payload.len() as u32);
    out.extend_from_slice(&payload);
    out
}

fn engine() -> Result<Engine> {
    let mut config = Config::new();
    config.strategy(Strategy::Cranelift);
    config.wasm_tail_call(true);
    config.rwasm_stack_limits(Some(RwasmStackLimits {
        max_call_depth: MAX_CALL_DEPTH,
        max_stack_slots: MAX_STACK_SLOTS,
        code_snippets: false,
    }));
    Engine::new(&config)
}

/// Runs `main(n)` from the counter state the rwasm executor sets before every call.
fn run(engine: &Engine, wasm: &[u8], n: i32) -> Result<i32> {
    let module = Module::new(engine, wasm)?;
    let mut store = Store::new(engine, ());
    store.set_rwasm_stack_counters(RwasmStackCounters {
        call_depth: 0,
        stack_slots: 0,
    });
    let instance = Instance::new(&mut store, &module, &[])?;
    let main = instance.get_typed_func::<i32, i32>(&mut store, "main")?;
    main.call(&mut store, n)
}

fn is_stack_overflow(err: &Error) -> bool {
    err.downcast_ref::<Trap>() == Some(&Trap::StackOverflow)
}

/// `main(n)` recurses `n` deep through plain calls: the emulated depth limit must trap.
fn recursion() -> Vec<u8> {
    let wasm = wat::parse_str(
        r#"(module
            (func $f (param i32) (result i32)
              (if (result i32) (i32.eqz (local.get 0))
                (then (i32.const 0))
                (else (i32.add (i32.const 1) (call $f (i32.sub (local.get 0) (i32.const 1)))))))
            (func (export "main") (param i32) (result i32) (call $f (local.get 0))))"#,
    )
    .unwrap();
    with_frame_heights(&wasm, &[4, 4])
}

/// `main(n)` recurses `n` deep through `return_call`, after a plain `call` in every frame:
/// the call publishes the callee's counters to the store and nothing restores them, so the
/// tail call must publish the caller's own again.
fn call_then_tail_recursion() -> Vec<u8> {
    let wasm = wat::parse_str(
        r#"(module
            (func $g (result i32) (i32.const 1))
            (func $f (param i32) (result i32)
              (drop (call $g))
              (if (result i32) (i32.eqz (local.get 0))
                (then (i32.const 0))
                (else (return_call $f (i32.sub (local.get 0) (i32.const 1))))))
            (func (export "main") (param i32) (result i32) (call $f (local.get 0))))"#,
    )
    .unwrap();
    with_frame_heights(&wasm, &[4, 4, 4])
}

/// The same chain through `return_call_indirect`.
fn call_then_tail_indirect_recursion() -> Vec<u8> {
    let wasm = wat::parse_str(
        r#"(module
            (type $t (func (param i32) (result i32)))
            (table 1 funcref)
            (elem (i32.const 0) $f)
            (func $g (result i32) (i32.const 1))
            (func $f (type $t)
              (drop (call $g))
              (if (result i32) (i32.eqz (local.get 0))
                (then (i32.const 0))
                (else (return_call_indirect (type $t)
                  (i32.sub (local.get 0) (i32.const 1)) (i32.const 0)))))
            (func (export "main") (param i32) (result i32) (call $f (local.get 0))))"#,
    )
    .unwrap();
    with_frame_heights(&wasm, &[4, 4, 4])
}

/// The control: `main` is the first frame, so `max_call_depth - 1` nested calls fit exactly and
/// one more traps `StackOverflow`.
#[test]
fn plain_recursion_traps_at_the_depth_limit() -> Result<()> {
    let engine = engine()?;
    let wasm = recursion();
    let deepest = MAX_CALL_DEPTH as i32 - 1;
    assert_eq!(run(&engine, &wasm, deepest)?, deepest);
    for depth in [deepest + 1, 2000] {
        let err = run(&engine, &wasm, depth).unwrap_err();
        assert!(is_stack_overflow(&err), "depth {depth}: {err:?}");
    }
    Ok(())
}

/// A tail call replaces the caller's frame, also after the frame made a plain call: the loop
/// runs far past the depth limit. Before 45.0.0-rwasm.4 the tail callee read the counters the
/// plain call had published for its callee, and the loop trapped after `max_call_depth`
/// iterations.
#[test]
fn tail_call_after_a_call_keeps_the_caller_frame() -> Result<()> {
    let engine = engine()?;
    let depth = 100 * MAX_CALL_DEPTH as i32;
    for (kind, wasm) in [
        ("return_call", call_then_tail_recursion()),
        ("return_call_indirect", call_then_tail_indirect_recursion()),
    ] {
        let result = run(&engine, &wasm, depth);
        assert!(
            matches!(result, Ok(0)),
            "{kind} after a call must run at the caller's depth: {result:?}"
        );
    }
    Ok(())
}
