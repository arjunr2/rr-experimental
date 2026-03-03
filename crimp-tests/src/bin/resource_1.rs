crimp_tests::bin!(@uses);

fn main() -> Result<()> {
    component_run::<_, RunTy, (), ()>(
        ComponentFmt::File("test-modules/components/resource_1.wasm"),
        |_| Ok(()),
        RunMode::InstantiateOnly,
    )
}
