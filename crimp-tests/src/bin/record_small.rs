crimp_tests::bin!(@uses);

bindgen!(
    "my-world" in "../test-modules/components/wit/record_small.wit"
);

impl component::test_package::env::Host for MyState {
    fn process_profile(&mut self, mut profile: UserProfile) -> UserProfile {
        // Modify some parameters
        // Convert username to uppercase
        profile.username = profile.username.to_uppercase();

        profile
    }
}

fn main() -> Result<()> {
    let input = UserProfile {
        id: 42,
        username: "arjunr2".into(),
        tags: vec!["coder".into(), "student".into()],
    };
    component_run::<_, RunTy, (UserProfile,), (String,)>(
        ComponentFmt::File("test-modules/components/record_small.wasm"),
        |mut linker| crimp_tests::bin!(@add linker, MyWorld),
        RunMode::InstantiateAndCallOnce {
            name: "main",
            params: (input,),
        },
    )
}

//fn main(mut profile: UserProfile) -> UserProfile {
//}
