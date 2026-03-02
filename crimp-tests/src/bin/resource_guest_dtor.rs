crimp_tests::bin!(@uses);

fn main() -> Result<()> {
    component_run::<_, RunTy, (), ()>(
        ComponentFmt::File("test-modules/components/resource_guest_dtor.wasm"),
        |_| Ok(()),
        RunMode::InstantiateAndCallOnce {
            name: "run",
            params: (),
        },
    )
}
