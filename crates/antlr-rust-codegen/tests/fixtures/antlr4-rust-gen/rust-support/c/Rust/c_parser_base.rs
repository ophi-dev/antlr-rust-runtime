#![allow(dead_code)]

use std::cell::Cell;

thread_local! {
    static DEPTH: Cell<usize> = const { Cell::new(0) };
}

pub fn enter_scope() {
    DEPTH.set(DEPTH.get() + 1);
}

pub fn exit_scope() {
    DEPTH.set(DEPTH.get().saturating_sub(1));
}

pub fn is_typedef_name(text: &str) -> bool {
    DEPTH.get() == 1 && text == "name"
}
