use fluentbase_types::compile_wasm_to_rwasm;
use wasmtime::{Engine, Instance, Module, Store};

#[test]
fn greeting_rwasm_binary()  -> anyhow::Result<()> {
    let greeting_binary = compile_wasm_to_rwasm(include_bytes!("./rwasm_files/greeting.wasm"))?;

    let module = Module::from_binary(&Engine::default(), &greeting_binary.rwasm_bytecode[..])?;

    let mut store = Store::new(module.engine(), ());
    let instance = Instance::new(&mut store, &module, &[])?;
    let memory = instance.get_memory(&mut store, "memory").unwrap();

    let mut bytes = [0; 12];
    memory.read(&store, 0, &mut bytes)?;
    assert_eq!(bytes, "Hello World!".as_bytes());
    Ok(())
}

#[test]
fn simple_rwasm_binary()  -> anyhow::Result<()> {
    let rwasm_module = compile_wasm_to_rwasm(include_bytes!("./rwasm_files/simple.wasm"))?.rwasm_bytecode;

    Ok(())
}
