use prime_factorization::Factorization;

impl component::test_package::env::Host for MyState {
    fn factors(&mut self, x: u32) -> Vec<u32> {
        Factorization::run(x).factors
    }
}

crimp_tests::bin!(@uses);

bindgen!(
    "my-world" in "../test-modules/components/wit/complex_params.wit"
);

fn main() -> Result<()> {
    component_run::<_, RunTy, (Vec<u32>,), (Vec<u32>,)>(
        ComponentFmt::File("test-modules/components/complex_params.wasm"),
        |mut linker| crimp_tests::bin!(@add linker, MyWorld),
        RunMode::InstantiateAndCallOnce {
            name: "main",
            params: ((0..10000).collect::<Vec<u32>>(),),
        },
    )
}
