impl component::test_package::env::Host for MyState {
    fn double(&mut self, x: u32) -> u32 {
        x * 2
    }

    fn complex(&mut self, x: u32, y: u64) -> u64 {
        (x * x) as u64 + y
    }
}

crimp_tests::bin!(@uses);

bindgen!(
    "root" in "../test-modules/components/wit/complex-singlereturn-indirect.wit"
);

fn main() -> Result<()> {
    component_run::<_, RunTy, (u32,), (u32,)>(
        ComponentFmt::File("test-modules/components/complex-singlereturn-indirect.wasm"),
        |mut linker| crimp_tests::bin!(@add linker, Root),
        RunMode::InstantiateAndCallOnce {
            name: "main",
            params: (42,),
        },
    )
}
