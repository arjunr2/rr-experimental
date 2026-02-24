#[allow(warnings)]
mod bindings;

use bindings::Guest;
use bindings::component::test_package::env::compute;

struct Component;

impl Guest for Component {
    fn main(
        a1: u32, a2: u32, a3: u32, a4: u32,
        a5: u32, a6: u32, a7: u32, a8: u32,
        a9: u32, a10: u32, a11: u32, a12: u32,
        a13: u32, a14: u32, a15: u32, a16: u32,
    ) -> (u32, u32) {
        compute(a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, a16)
    }
}

bindings::export!(Component with_types_in bindings);
