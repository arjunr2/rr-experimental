impl component::test_package::env::Host for MyState {
    fn compute(
        &mut self,
        a1: u32, a2: u32, a3: u32, a4: u32,
        a5: u32, a6: u32, a7: u32, a8: u32,
        a9: u32, a10: u32, a11: u32, a12: u32,
        a13: u32, a14: u32, a15: u32, a16: u32,
        a17: u32,
    ) -> (u32, u32) {
        let sum = a1 + a2 + a3 + a4 + a5 + a6 + a7 + a8
            + a9 + a10 + a11 + a12 + a13 + a14 + a15 + a16 + a17;
        let product = a1.wrapping_mul(a2).wrapping_mul(a3).wrapping_mul(a4);
        (sum, product)
    }
}

crimp_tests::bin!(@uses);

bindgen!(
    "my-world" in "../test-modules/components/wit/over_max_flat.wit"
);

fn main() -> Result<()> {
    component_run::<_, RunTy, (u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32), ((u32, u32),)>(
        ComponentFmt::File("test-modules/components/over_max_flat.wasm"),
        |mut linker| crimp_tests::bin!(@add linker, MyWorld),
        RunMode::InstantiateAndCallOnce {
            name: "main",
            params: (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17),
        },
    )
}
