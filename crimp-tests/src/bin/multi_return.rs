use prime_factorization::Factorization;

impl component::test_package::env::Host for MyState {
    fn factors(&mut self, x: u32) -> Vec<u32> {
        Factorization::run(x).factors
    }
}

crimp_tests::bin!(@uses);

bindgen!(
    "my-world" in "../test-modules/components/wit/multi_return.wit"
);

fn main() -> Result<()> {
    component_run::<_, RunTy, (u32,), (u32,)>(
        ComponentFmt::File("test-modules/components/multi_return.wasm"),
        |mut linker| crimp_tests::bin!(@add linker, MyWorld),
        RunMode::InstantiateAndCallNTimes {
            name: "main",
            params: (42,),
            n: 3,
        },
    )
}
