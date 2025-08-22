use anyhow;
use wasmtime::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::new();
    config.consume_fuel(true);
    config.wasm_backtrace(true);
    let engine = Engine::new(&config)?;
    let mut store = Store::new(&engine, ());
    store.set_fuel(1000)?;
    store.set_pause_execution_no_unwind();
    let mut linker = Linker::new(&engine);
    linker.func_wrap(
        "host",
        "pause",
        |mut caller: Caller<'_, ()>| -> anyhow::Result<()> {
            println!("Host: triggering pause...");
            match caller.pause_execution() {
                Ok(()) => {
                    println!("Host: pause_execution returned Ok - execution paused successfully");
                    Ok(())
                }
                Err(trap) => {
                    println!("Host: Got trap {}", trap);
                    Err(trap.into())
                }
            }
        },
    )?;

    let wasm = wat::parse_str(
        r#"
        (module
            (import "host" "pause" (func $pause))
            (func $helper_function (result i32)
                call $pause
                i32.const 200
            )
            (func (export "call_pause") (result i32)
                call $helper_function
                i32.const 100
                i32.add
                return
            )
            (global $counter (mut i32) (i32.const 42))
            (global $limit i32 (i32.const 1000))
            (memory 1)
        )
    "#,
    )?;

    let module = Module::new(&engine, &wasm)?;
    let instance = linker.instantiate(&mut store, &module)?;
    let call_pause_func = instance.get_typed_func::<(), i32>(&mut store, "call_pause")?;

    println!("Calling WASM function that calls host function...");
    match call_pause_func.call(&mut store, ()) {
        Ok(result) => {
            println!("Wasm function completed with result: {}", result);

            // Check if execution was paused during the call
            if store.is_execution_paused() {
                println!("Execution was paused during the call!");
                if let Some(handle) = store.capture_execution_handle() {
                    let paused_state = handle.paused_state();
                    println!("pc: 0x{:x}", paused_state.pc);
                    println!("fp: 0x{:x}", paused_state.fp);
                    if paused_state.pc == 1 && paused_state.fp == 1 {
                        println!(
                            "Note: pc=0x1, fp=0x1 indicates pause from host function (expected)"
                        );
                    }

                    // Fuel is tracked in the store itself, not in the paused state
                    if let Ok(fuel) = store.get_fuel() {
                        println!("Fuel remaining: {}", fuel);
                    }

                    println!("\nResuming execution with return value 42...");
                    match handle.resume(&mut store, vec![Val::I32(42)]) {
                        Ok(values) => {
                            println!("Resume returned {} values", values.len());
                            if let Some(Val::I32(val)) = values.first() {
                                println!("First return value: {}", val);
                            }
                        }
                        Err(e) => {
                            println!("Resume failed: {}", e);
                        }
                    }
                } else {
                    println!(
                        " No execution handle available (capture_execution_handle returned None)"
                    );
                }
            } else {
                println!("Execution was not paused during the call");
            }
        }
        Err(trap) => {
            println!("Caught trap: {}", trap);
        }
    }
    Ok(())
}
