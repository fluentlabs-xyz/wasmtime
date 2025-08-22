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

    linker.func_wrap(
        "host",
        "pause",
        move |mut caller: Caller<'_, ()>| -> anyhow::Result<()> {
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
        },
    )?;

    let wasm = wat::parse_str(
        r#"
        (module
            (import "host" "pause" (func $pause))
            (func (export "test_func") (result i32)
                call $pause
                i32.const 42
            )
        )
    "#,
    )?;

    let module = Module::new(&engine, &wasm)?;
    let instance = linker.instantiate(&mut store, &module)?;
    let test_func = instance.get_typed_func::<(), i32>(&mut store, "test_func")?;

    // First call should pause execution
    let result = test_func.call(&mut store, ());
    assert!(
        result.is_err(),
        "Function call should result in error due to pause"
    );
    let trap = result.unwrap_err();
    assert!(
        trap.to_string().contains("execution paused"),
        "Trap should indicate execution was paused"
    );

    // Verify we can get an execution handle
    assert!(
        store.is_execution_paused(),
        "Store should indicate execution is paused"
    );

    let handle = store.capture_execution_handle();
    assert!(
        handle.is_some(),
        "Should be able to capture execution handle when paused"
    );

    if let Some(handle) = handle {
        // Verify handle has valid paused state
        let paused_state = handle.paused_state();
        assert!(
            paused_state.pc != 0 || paused_state.fp != 0,
            "Paused state should have valid PC or FP"
        );

        // Attempt to resume - this should complete the function
        let resume_result = handle.resume(&mut store);
        assert!(resume_result.is_ok(), "Resume should succeed");

        if let Ok(values) = resume_result {
            assert!(!values.is_empty(), "Resume should return values");
            if let Some(Val::I32(result)) = values.first() {
                assert_eq!(
                    *result, 42,
                    "Function should return expected value after resume"
                );
            } else {
                panic!("Resume should return i32 value");
            }
        }

        assert!(
            !store.is_execution_paused(),
            "Store should no longer be paused after resume"
        );
    }

    Ok(())
}

#[test]
fn test_instance_specific_execution_handles() {
    use std::sync::{Arc, Mutex};

    let engine = Engine::default();
    let mut store = Store::new(&engine, ());
    store.set_pause_execution_no_unwind();

    let call_count = Arc::new(Mutex::new(0));
    let call_count_clone = call_count.clone();

    // Host function that pauses on first call
    let host_func = Func::wrap(
        &mut store,
        move |mut caller: Caller<'_, ()>| -> anyhow::Result<i32> {
            let mut count = call_count_clone.lock().unwrap();
            *count += 1;
            if *count == 1 {
                caller.pause_execution()?;
                Ok(0) // Won't be reached
            } else {
                Ok(42)
            }
        },
    );

    let wat = r#"
        (module
            (import "host" "pause_func" (func $pause (result i32)))
            (func (export "pausable_func") (result i32)
                i32.const 100
                call $pause
                i32.add
            )
        )
    "#;

    let module = Module::new(&engine, wat).unwrap();

    // Create two instances of the same module
    let instance1 = Instance::new(&mut store, &module, &[host_func.into()]).unwrap();
    let instance2 = Instance::new(&mut store, &module, &[host_func.into()]).unwrap();
    let func1 = instance1
        .get_typed_func::<(), i32>(&mut store, "pausable_func")
        .unwrap();

    // Call function from instance1 - this should pause
    let result = func1.call(&mut store, ());
    match result {
        Err(trap) if trap.to_string().contains("execution paused") => {
            // Check if we can get execution handle from instance1
            let handle1 = instance1.get_execution_handle(&mut store);
            assert!(
                handle1.is_some(),
                "Should get execution handle from paused instance"
            );

            // Check if we can get execution handle from instance2 (should be None)
            let handle2 = instance2.get_execution_handle(&mut store);
            assert!(
                handle2.is_none(),
                "Should not get execution handle from non-paused instance"
            );

            // Check store-level handle
            assert!(
                store.is_execution_paused(),
                "Store should have paused execution"
            );

            // Resume and verify
            if let Some(handle) = handle1 {
                let resume_result = handle.resume(&mut store);
                assert!(resume_result.is_ok(), "Resume should succeed");

                // After resume, no instance should have execution handles
                assert!(
                    instance1.get_execution_handle(&mut store).is_none(),
                    "Instance1 should not have execution handle after resume"
                );
                assert!(
                    instance2.get_execution_handle(&mut store).is_none(),
                    "Instance2 should not have execution handle after resume"
                );
                assert!(
                    !store.is_execution_paused(),
                    "Store should not be paused after resume"
                );
            }
        }
        _ => panic!("Function should have paused"),
    }
}

#[test]
fn test_function_restart_analysis() {
    use std::sync::{Arc, Mutex};

    let engine = Engine::default();
    let mut store = Store::new(&engine, ());
    store.set_pause_execution_no_unwind();

    let call_count = Arc::new(Mutex::new(0));
    let call_count_clone = call_count.clone();

    let function_call_count = Arc::new(Mutex::new(0));
    let function_call_count_clone = function_call_count.clone();

    let host_func = Func::wrap(
        &mut store,
        move |mut caller: Caller<'_, ()>| -> anyhow::Result<i32> {
            let mut count = call_count_clone.lock().unwrap();
            *count += 1;

            if *count == 1 {
                caller.pause_execution()?;
                Ok(0) // This won't be reached due to pause
            } else {
                Ok(999)
            }
        },
    );

    let wat = r#"
        (module
            (import "host" "pause_and_return" (func $pause (result i32)))
            (import "host" "increment_counter" (func $increment))
            (func (export "test_restart") (result i32)
                call $increment

                i32.const 100
                i32.const 200
                i32.add

                call $pause

                i32.add
            )
        )
    "#;

    let increment_func = Func::wrap(
        &mut store,
        move |_caller: Caller<'_, ()>| -> anyhow::Result<()> {
            let mut count = function_call_count_clone.lock().unwrap();
            *count += 1;
            Ok(())
        },
    );

    let module = Module::new(&engine, wat).unwrap();
    let instance = Instance::new(
        &mut store,
        &module,
        &[host_func.into(), increment_func.into()],
    )
    .unwrap();

    let test_func = instance
        .get_typed_func::<(), i32>(&mut store, "test_restart")
        .unwrap();

    let result = test_func.call(&mut store, ());
    match result {
        Err(trap) if trap.to_string().contains("execution paused") => {
            let function_calls_after_pause = *function_call_count.lock().unwrap();
            assert_eq!(
                function_calls_after_pause, 1,
                "Function body should be executed exactly once before pause"
            );

            assert!(
                store.is_execution_paused(),
                "Store should be in paused state"
            );

            if let Some(handle) = store.capture_execution_handle() {
                let paused_state = handle.paused_state();
                assert!(
                    paused_state.pc != 0 || paused_state.fp != 0,
                    "Should have valid paused state"
                );

                let resume_result = handle.resume(&mut store);
                assert!(resume_result.is_ok(), "Resume should succeed");

                let total_function_calls = *function_call_count.lock().unwrap();
                let host_calls = *call_count.lock().unwrap();

                // Critical assertion: function body should only be executed once (true resume)
                assert_eq!(
                    total_function_calls, 1,
                    "Function body should only be executed once - indicating true resume, not restart"
                );

                // Host function should be called twice (pause + resume)
                assert_eq!(
                    host_calls, 2,
                    "Host function should be called twice - once for pause, once for resume"
                );

                // Verify the actual result if resume succeeded
                if let Ok(values) = resume_result {
                    if let Some(Val::I32(result)) = values.first() {
                        // Expected: 300 (from 100+200) + 999 (from resumed host call) = 1299
                        assert_eq!(
                            *result, 1299,
                            "Resume should produce correct computation result"
                        );
                    }
                }
            }
        }
        Ok(_) => panic!("Function should have paused, not completed immediately"),
        Err(e) => panic!("Unexpected error: {}", e),
    }
}

#[test]
fn test_register_state_analysis() {
    use std::sync::{Arc, Mutex};

    let engine = Engine::default();
    let mut store = Store::new(&engine, ());
    store.set_pause_execution_no_unwind();

    let call_count = Arc::new(Mutex::new(0));
    let call_count_clone = call_count.clone();

    let host_func = Func::wrap(
        &mut store,
        move |mut caller: Caller<'_, ()>| -> anyhow::Result<i32> {
            let mut count = call_count_clone.lock().unwrap();
            *count += 1;

            if *count == 1 {
                caller.pause_execution()?;
                Ok(0) // Won't be reached
            } else {
                Ok(42)
            }
        },
    );

    let wat = r#"
        (module
            (import "host" "pause_and_return" (func $pause (result i32)))
            (func (export "test_locals") (result i32)
                (local $a i32)
                (local $b i32)
                (local $c i32)
                (local $d i32)

                i32.const 100
                local.set $a

                i32.const 200
                local.set $b

                i32.const 300
                local.set $c

                local.get $a
                local.get $b
                i32.add
                local.set $d

                call $pause

                local.get $d
                i32.add

                local.get $c
                i32.add
            )
        )
    "#;

    let module = Module::new(&engine, wat).unwrap();
    let instance = Instance::new(&mut store, &module, &[host_func.into()]).unwrap();

    let test_func = instance
        .get_typed_func::<(), i32>(&mut store, "test_locals")
        .unwrap();

    let result = test_func.call(&mut store, ());
    match result {
        Err(trap) if trap.to_string().contains("execution paused") => {
            assert!(
                store.is_execution_paused(),
                "Store should be in paused state"
            );

            if let Some(handle) = store.capture_execution_handle() {
                let resume_result = handle.resume(&mut store);
                assert!(resume_result.is_ok(), "Resume should succeed");

                // Expected: 300 (d) + 42 (host return) + 300 (c) = 642 if locals preserved
                match resume_result {
                    Ok(values) => {
                        assert!(!values.is_empty(), "Resume should return values");
                        if let Some(Val::I32(result)) = values.first() {
                            assert_eq!(
                                *result, 642,
                                "Result should be 642 if local variables are preserved across pause/resume"
                            );
                        } else {
                            panic!("Resume should return i32 value");
                        }
                    }
                    Err(e) => panic!("Resume should not fail: {}", e),
                }
            }
        }
        Ok(_) => panic!("Function should have paused, not completed immediately"),
        Err(e) => panic!("Unexpected error: {}", e),
    }
}

#[test]
fn test_wasm_stack_analysis() {
    use std::sync::{Arc, Mutex};

    let engine = Engine::default();
    let mut store = Store::new(&engine, ());
    store.set_pause_execution_no_unwind();

    let call_count = Arc::new(Mutex::new(0));
    let call_count_clone = call_count.clone();

    let host_func = Func::wrap(
        &mut store,
        move |mut caller: Caller<'_, ()>| -> anyhow::Result<()> {
            let mut count = call_count_clone.lock().unwrap();
            *count += 1;

            if *count == 1 {
                caller.pause_execution()?;
            }
            Ok(())
        },
    );

    let wat = r#"
        (module
            (import "host" "pause" (func $pause))
            (func (export "test_stack") (result i32)
                i32.const 111
                i32.const 222
                i32.const 333
                i32.const 444

                call $pause

                i32.add
                i32.add
                i32.add
            )
        )
    "#;

    let module = Module::new(&engine, wat).unwrap();
    let instance = Instance::new(&mut store, &module, &[host_func.into()]).unwrap();

    let test_func = instance
        .get_typed_func::<(), i32>(&mut store, "test_stack")
        .unwrap();

    let result = test_func.call(&mut store, ());
    match result {
        Err(trap) if trap.to_string().contains("execution paused") => {
            assert!(
                store.is_execution_paused(),
                "Store should be in paused state"
            );

            if let Some(handle) = store.capture_execution_handle() {
                let resume_result = handle.resume(&mut store);
                assert!(resume_result.is_ok(), "Resume should succeed");

                // Expected: 111 + 222 + 333 + 444 = 1110 if stack preserved
                match resume_result {
                    Ok(values) => {
                        assert!(!values.is_empty(), "Resume should return values");
                        if let Some(Val::I32(result)) = values.first() {
                            assert_eq!(
                                *result, 1110,
                                "Result should be 1110 if operand stack is preserved across pause/resume"
                            );
                        } else {
                            panic!("Resume should return i32 value");
                        }
                    }
                    Err(e) => panic!("Resume should not fail: {}", e),
                }
            }
        }
        Ok(_) => panic!("Function should have paused, not completed immediately"),
        Err(e) => panic!("Unexpected error: {}", e),
    }
}
