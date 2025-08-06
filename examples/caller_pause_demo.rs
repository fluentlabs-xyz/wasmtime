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
    linker.func_wrap("host", "pause", |mut caller: Caller<'_, ()>| -> anyhow::Result<()> {
        println!("Host: triggering pause...");
        match caller.pause_execution() {
            Ok(()) => {
                println!("Host: pause_execution returned Ok - unexpected!");
                Ok(())
            }
            Err(trap) => {
                println!("Host: Got trap {}", trap);
                Err(trap.into())
            }
        }
    })?;

    let wasm = wat::parse_str(r#"
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
    "#)?;

    let module = Module::new(&engine, &wasm)?;
    let instance = linker.instantiate(&mut store, &module)?;
    let call_pause_func = instance.get_typed_func::<(), i32>(&mut store, "call_pause")?;

    println!("Calling WASM function that calls host function...");
    match call_pause_func.call(&mut store, ()) {
        Ok(result) => {
            println!("Unexpected success: {}", result);
        }
        Err(trap) => {
            println!("Caught trap: {}", trap);
            if store.is_execution_paused() {
                println!("Execution is paused");
                match store.get_paused_state() {
                    Some(paused_state) => {
                        println!("pc: 0x{:x}", paused_state.pc);
                        println!("fp: 0x{:x}", paused_state.fp);
                        if let Some(fuel) = paused_state.fuel_remaining {
                            println!("Fuel remaining: {}", fuel);
                        }
                        // TODO: remove this?
                        println!("Call stack frames: {}", paused_state.call_stack.len());
                        for (i, frame) in paused_state.call_stack.iter().enumerate() {
                            println!("  Frame {}: {:?} @ offset 0x{:x}",
                                   i, frame.function_name.as_deref().unwrap_or("<unknown>"),
                                   frame.instruction_offset);
                        }
                    }
                    None => {
                        println!("⚠️  No paused state available (get_paused_state returned None)");
                    }
                }
                // Clear paused state
                store.clear_paused_state();
            } else {
                println!("Execution is not paused (we got a different trap?)");
                println!("Trap: {:?}", trap);
            }
        }
    }
    Ok(())
}
