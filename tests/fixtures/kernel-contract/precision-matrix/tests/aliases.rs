use std::{dbg as aliased_dbg, todo as aliased_todo, unimplemented as aliased_unimplemented};

fn aliased_dbg_case(value: u8) -> u8 {
    aliased_dbg!(value)
}

fn aliased_todo_case() -> u8 {
    aliased_todo!("aliased")
}

fn aliased_unimplemented_case() -> u8 {
    aliased_unimplemented!("aliased")
}

#[test]
fn aliases_compile_as_test_target_cases() {
    let _ = (
        aliased_dbg_case as fn(u8) -> u8,
        aliased_todo_case as fn() -> u8,
        aliased_unimplemented_case as fn() -> u8,
    );
}
