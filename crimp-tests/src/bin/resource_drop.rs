crimp_tests::bin!(@uses);

use wasmtime::component::Resource;

pub struct Counter {
    count: u32,
}

bindgen!({
    world: "my-world",
    path: "../test-modules/components/wit/resource_drop.wit",
    with: {
        "component:test-resources/env.counter": Counter,
    },
});

impl component::test_resources::env::HostCounter for MyState {
    fn increment(&mut self, self_: Resource<Counter>) {
        let counter = self.table.get_mut(&self_).unwrap();
        counter.count += 1;
    }

    fn drop(&mut self, rep: Resource<Counter>) -> Result<()> {
        self.table.delete(rep)?;
        Ok(())
    }
}

impl component::test_resources::env::Host for MyState {
    fn create_counter(&mut self) -> Resource<Counter> {
        self.table.push(Counter { count: 0 }).unwrap()
    }

    fn ping(&mut self, n: u32) -> u32 {
        n
    }
}

fn main() -> Result<()> {
    component_run::<_, RunTy, (), ()>(
        ComponentFmt::File("test-modules/components/resource_drop.wasm"),
        |mut linker| crimp_tests::bin!(@add linker, MyWorld),
        RunMode::InstantiateAndCallOnce {
            name: "run",
            params: (),
        },
    )
}
