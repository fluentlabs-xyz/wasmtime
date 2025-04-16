(module $fluentbase_examples_greeting.wasm
  (type (;0;) (func (param i32 i32)))
  (type (;1;) (func))
  (import "fluentbase_v1preview" "_write" (func $_ZN14fluentbase_sdk8bindings6_write17ha5a1d7572619bf51E (type 0)))
  (func $main (type 1)
    i32.const 131072
    i32.const 12
    call $_ZN14fluentbase_sdk8bindings6_write17ha5a1d7572619bf51E)
  (func $deploy (type 1))
  (memory (;0;) 3)
  (global $__stack_pointer (mut i32) (i32.const 131072))
  (global (;1;) i32 (i32.const 131084))
  (global (;2;) i32 (i32.const 131088))
  (export "memory" (memory 0))
  (export "main" (func $main))
  (export "deploy" (func $deploy))
  (export "__data_end" (global 1))
  (export "__heap_base" (global 2))
  (data $.rodata (i32.const 131072) "Hello, World"))
