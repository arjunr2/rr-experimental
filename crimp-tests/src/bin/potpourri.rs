crimp_tests::bin!(@uses);

bindgen!(
    "my-world" in "../test-modules/components/wit/potpourri.wit"
);

use component::test_package::env::{PaymentMethod, PotPayload, SmallRecord};

impl component::test_package::env::Host for MyState {
    fn process_pot(&mut self, mut p: PotRecord) -> PotRecord {
        // Simple transformations to exercise types
        p.id = p.id + 1;
        p.name = p.name.to_uppercase();
        p.active = !p.active;
        p.score += 1.0;

        match p.opt_tags.as_mut() {
            Some(tags) => {
                tags.push("processed".to_string());
            }
            None => {
                p.opt_tags = Some(vec!["processed".to_string()]);
            }
        }

        p.payload = match p.payload {
            PotPayload::Numbers(mut nums) => {
                for n in nums.iter_mut() {
                    *n = *n + 10;
                }
                PotPayload::Numbers(nums)
            }
            PotPayload::Small(mut s) => {
                s.count += 1;
                s.ratio *= 1.05;
                PotPayload::Small(s)
            }
            PotPayload::Payment(pm) => {
                let transformed = match pm {
                    // mask credit card
                    PaymentMethod::CreditCard(mut c) => {
                        if c.len() > 4 {
                            c = format!("****{}", &c[c.len() - 4..]);
                        }
                        PaymentMethod::CreditCard(c)
                    }
                    PaymentMethod::Paypal(e) => PaymentMethod::Paypal(e.to_lowercase()),
                    PaymentMethod::Cash => PaymentMethod::Cash,
                };
                PotPayload::Payment(transformed)
            }
            PotPayload::Text(t) => PotPayload::Text(format!("[HOST] {}", t)),
        };

        // Toggle the validation result
        p.validation = match p.validation {
            Result::Ok(msg) => Result::Ok(format!("validated: {}", msg)),
            Result::Err(err) => Result::Err(format!("error handled: {}", err)),
        };

        p
    }
}

fn main() -> Result<()> {
    use std::result::Result as StdResult;

    let input = PotRecord {
        id: 7,
        name: "potpourri".to_string(),
        active: true,
        score: 3.14,
        opt_tags: Some(vec!["alpha".to_string(), "beta".to_string()]),
        payload: PotPayload::Small(SmallRecord {
            count: 2,
            ratio: 0.5,
        }),
        validation: StdResult::Ok("initial state".to_string()),
    };

    component_run::<_, RunTy, (PotRecord,), (String,)>(
        ComponentFmt::File("test-modules/components/potpourri.wasm"),
        |mut linker| crimp_tests::bin!(@add linker, MyWorld),
        RunMode::InstantiateAndCallOnce {
            name: "main",
            params: (input,),
        },
    )
}
