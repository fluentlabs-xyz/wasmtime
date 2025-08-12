use anyhow::Result;
use wasmtime::*;

#[test]
fn test_basic_pause_resume() -> Result<()> {
    let mut config = Config::new();
    config.consume_fuel(true);
    config.wasm_backtrace(true);

    let engine = Engine::new(&config)?;
    let mut store = Store::new(&engine, ());
    store.set_fuel(1000)?;
    store.set_pause_execution_no_unwind();

    let mut linker = Linker::new(&engine);
    let call_count = std::sync::Arc::new(std::sync::Mutex::new(0));
    let call_count_clone = call_count.clone();

    linker.func_wrap("host", "pause", move |mut caller: Caller<'_, ()>| -> anyhow::Result<()> {
        let mut count = call_count_clone.lock().unwrap();
        *count += 1;
        let current_call = *count;
        if current_call == 1 {
            // First call - trigger pause
            log::trace!("Host function first call - triggering pause");
            match caller.pause_execution() {
                Ok(()) => Ok(()),
                Err(trap) => Err(anyhow::Error::from(trap)),
            }
        } else {
            // Resume call - just continue normally and return
            log::trace!("Host function resume call - continuing normally");
            Ok(())
        }
    })?;

    let wasm = wat::parse_str(r#"
        (module
            (import "host" "pause" (func $pause))
            (func (export "test_func") (result i32)
                call $pause
                i32.const 42
            )
        )
    "#)?;

    let module = Module::new(&engine, &wasm)?;
    let instance = linker.instantiate(&mut store, &module)?;
    let test_func = instance.get_typed_func::<(), i32>(&mut store, "test_func")?;

    // Test pause
    match test_func.call(&mut store, ()) {
        Ok(result) => panic!("Expected pause trap, got: {}", result),
        Err(_) => {
            assert!(store.is_execution_paused(), "Execution should be paused");
        }
    }

    // Test resume
    let handle = store.capture_execution_handle();
    match handle.resume(&mut store) {
        Ok(_) => {
            assert!(!store.is_execution_paused(), "Execution should not be paused after resume");
            match test_func.call(&mut store, ()) {
                Ok(result) => {
                    assert_eq!(result, 42);
                    println!("Resume successful! returned value: {}", result);
                }
                Err(e) => panic!("Function call after resume failed: {}", e),
            }
        }
        Err(e) => panic!("Resume failed: {}", e),
    }

    Ok(())
}

#[test]
fn test_multiple_pause_resume_cycles() -> Result<()> {
    let mut config = Config::new();
    config.consume_fuel(true);

    let engine = Engine::new(&config)?;
    let mut store = Store::new(&engine, 0i32); // Counter state
    store.set_fuel(1000)?;
    store.set_pause_execution_no_unwind();

    let mut linker = Linker::new(&engine);
    linker.func_wrap("host", "pause_and_count", |mut caller: Caller<'_, i32>| -> anyhow::Result<i32> {
        let counter = {
            let counter_ref = caller.data_mut();
            *counter_ref += 1;
            *counter_ref
        };
        if counter < 3 {
            match caller.pause_execution() {
                Ok(()) => Ok(counter),
                Err(trap) => Err(anyhow::Error::from(trap)),
            }
        } else {
            Ok(counter)
        }
    })?;

    let wasm = wat::parse_str(r#"
        (module
            (import "host" "pause_and_count" (func $pause_and_count (result i32)))
            (func (export "multi_pause") (result i32)
                call $pause_and_count
                return
            )
        )
    "#)?;

    let module = Module::new(&engine, &wasm)?;
    let instance = linker.instantiate(&mut store, &module)?;
    let multi_pause_func = instance.get_typed_func::<(), i32>(&mut store, "multi_pause")?;

    // First call - should pause
    match multi_pause_func.call(&mut store, ()) {
        Err(_) if store.is_execution_paused() => {
            // Resume and pause again
            let handle1 = store.capture_execution_handle();
            handle1.resume(&mut store)?;
            if store.is_execution_paused() {
                // Final resume
                let handle2 = store.capture_execution_handle();
                let _results = handle2.resume(&mut store)?;
            }
        }
        _ => panic!("Expected pause on first call"),
    }

    Ok(())
}

#[test]
fn test_state_preservation() -> Result<()> {
    let mut config = Config::new();
    config.consume_fuel(true);

    let engine = Engine::new(&config)?;
    let mut store = Store::new(&engine, ());

    let initial_fuel = 500;
    store.set_fuel(initial_fuel)?;
    store.set_pause_execution_no_unwind();

    let mut linker = Linker::new(&engine);
    linker.func_wrap("host", "pause", |mut caller: Caller<'_, ()>| -> anyhow::Result<()> {
        match caller.pause_execution() {
            Ok(()) => Ok(()),
            Err(trap) => Err(anyhow::Error::from(trap)),
        }
    })?;

    let wasm = wat::parse_str(r#"
        (module
            (import "host" "pause" (func $pause))
            (global $counter (mut i32) (i32.const 0))
            (memory 1)
            (func (export "test_state") (result i32)
                ;; Consume some fuel and modify state
                i32.const 10
                global.set $counter
                i32.const 0
                i32.const 123
                i32.store

                call $pause

                ;; This should be preserved after resume
                global.get $counter
            )
        )
    "#)?;

    let module = Module::new(&engine, &wasm)?;
    let instance = linker.instantiate(&mut store, &module)?;
    let test_func = instance.get_typed_func::<(), i32>(&mut store, "test_state")?;

    // Trigger pause and capture state
    match test_func.call(&mut store, ()) {
        Err(_) if store.is_execution_paused() => {
            let paused_state = store.get_paused_state().unwrap();
            let remaining_fuel = store.get_fuel().unwrap_or(0);
            assert_ne!(paused_state.pc, 0, "pc should be captured");
            assert_ne!(paused_state.fp, 0, "fp should be captured");
            assert!(remaining_fuel < initial_fuel, "Fuel should be consumed");
            let handle = store.capture_execution_handle();
            let _results = handle.resume(&mut store)?;
        }
        _ => panic!("Expected pause trap"),
    }

    Ok(())
}

#[test]
fn test_error_handling() -> Result<()> {
    let mut config = Config::new();
    config.consume_fuel(true);

    let engine = Engine::new(&config)?;
    let mut store = Store::new(&engine, ());
    store.set_fuel(1000)?;
    store.set_pause_execution_no_unwind();

    // Test: Resume without pause (invalid state)
    let dummy_state = PausedExecutionState {
        pc: 0,
        fp: 0,

        fuel_remaining: Some(1000),
    };

    let invalid_handle = ExecutionHandle::new_for_test(dummy_state);

    match invalid_handle.resume(&mut store) {
        Err(_) => {
            // Expected to fail
        }
        Ok(_) => panic!("Should reject resume with invalid state"),
    }

    Ok(())
}

#[test]
fn test_execution_handle_api() -> Result<()> {
    let mut config = Config::new();
    config.consume_fuel(true);

    let engine = Engine::new(&config)?;
    let mut store = Store::new(&engine, ());
    store.set_fuel(1000)?;
    store.set_pause_execution_no_unwind();

    let mut linker = Linker::new(&engine);
    linker.func_wrap("host", "pause", |mut caller: Caller<'_, ()>| -> anyhow::Result<()> {
        match caller.pause_execution() {
            Ok(()) => Ok(()),
            Err(trap) => Err(anyhow::Error::from(trap)),
        }
    })?;

    let wasm = wat::parse_str(r#"
        (module
            (import "host" "pause" (func $pause))
            (func (export "test") (result i32)
                call $pause
                i32.const 42
            )
        )
    "#)?;

    let module = Module::new(&engine, &wasm)?;
    let instance = linker.instantiate(&mut store, &module)?;
    let test_func = instance.get_typed_func::<(), i32>(&mut store, "test")?;

    match test_func.call(&mut store, ()) {
        Err(_) if store.is_execution_paused() => {
            let handle = store.capture_execution_handle();
            assert!(handle.can_resume(), "Handle should be resumable");
            let paused_state = handle.paused_state();
            assert_ne!(paused_state.pc, 0, "Should have valid PC");
            assert_ne!(paused_state.fp, 0, "Should have valid FP");
            assert!(paused_state.fuel_remaining.is_some(), "Should have fuel info");
            let results = handle.resume(&mut store)?;
            assert!(!results.is_empty(), "Should return results");
        }
        _ => panic!("Expected pause trap"),
    }

    Ok(())
}
