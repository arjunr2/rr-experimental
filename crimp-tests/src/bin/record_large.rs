crimp_tests::bin!(@uses);

bindgen!(
    "my-world" in "../test-modules/components/wit/record_large.wit"
);

impl component::test_package::env::Host for MyState {
    fn process_profile(&mut self, mut profile: UserProfile) -> UserProfile {
        // Modify some parameters
        // Convert username to uppercase
        profile.username = profile.username.to_uppercase();

        // Mark as verified
        profile.is_verified = true;

        // Increase loyalty points by 100
        profile.loyalty_points += 100;

        // Apply a bonus to account balance (10% increase)
        profile.account_balance *= 1.10;

        // Upgrade subscription tier if conditions met
        if profile.loyalty_points > 500 && profile.subscription_tier == "basic" {
            profile.subscription_tier = "premium".to_string();
        }

        // Update last login to current timestamp (simulated)
        profile.last_login_timestamp = 1699200000; // Example timestamp

        // Modify lists

        // Add a new tag
        profile.tags.push("verified-user".to_string());

        // Filter out duplicate tags
        profile.tags.sort();
        profile.tags.dedup();

        // Add a new purchase ID to history
        profile.purchase_history.push(987654321);

        // Keep only last 10 purchases
        let num_purchases = profile.purchase_history.len();
        if num_purchases > 10 {
            profile.purchase_history = profile
                .purchase_history
                .into_iter()
                .skip(num_purchases - 10)
                .collect();
        }

        // Add a preference if not already present
        if !profile
            .preferences
            .contains(&"email-notifications".to_string())
        {
            profile.preferences.push("email-notifications".to_string());
        }

        profile
    }
}

fn main() -> Result<()> {
    let input = UserProfile {
        id: 42,
        username: "arjunr2".into(),
        email: "arjunr2@andrew.cmu.edu".into(),
        first_name: "Arjun".into(),
        last_name: "Ramesh".into(),
        age: 37,
        country: "Bahamas".into(),
        city: "idk".into(),
        postal_code: "17683".into(),
        phone: "9992228367".into(),
        is_verified: false,
        account_balance: 3.0,
        loyalty_points: 10000,
        subscription_tier: "none".into(),
        last_login_timestamp: 15,
        registration_date: "never".into(),
        tags: vec!["coder".into(), "wasting time".into()],
        purchase_history: (0..20).into_iter().map(|x| 900 - x).collect::<Vec<_>>(),
        preferences: vec!["none".into()],
    };
    component_run::<_, RunTy, (UserProfile,), (String,)>(
        ComponentFmt::File("test-modules/components/record_large.wasm"),
        |mut linker| crimp_tests::bin!(@add linker, MyWorld),
        RunMode::InstantiateAndCallOnce {
            name: "main",
            params: (input,),
        },
    )
}

//fn main(mut profile: UserProfile) -> UserProfile {
//}
