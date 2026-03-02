crimp_tests::bin!(@uses);

fn main() -> Result<()> {
    component_run::<_, RunTy, (), ()>(
        ComponentFmt::File("test-modules/components/resource-3.wasm"),
        |_| Ok(()),
        RunMode::InstantiateAndCallOnce {
            name: "main",
            params: (),
        },
    )
}
