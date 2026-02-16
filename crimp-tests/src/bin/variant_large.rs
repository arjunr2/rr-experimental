crimp_tests::bin!(@uses);

bindgen!(
    "my-world" in "../test-modules/components/wit/variant_large.wit"
);

use component::test_package::env::{Address, OrderInfo, PaymentMethod};

impl component::test_package::env::Host for MyState {
    fn process_data(&mut self, data: DataPayload) -> DataPayload {
        // Process and transform the variant based on its type
        match data {
            DataPayload::Order(mut order) => {
                // Apply a discount to the order
                order.total_amount *= 0.90; // 10% discount

                // Add a promotional item
                order.items.push("FREE-SHIPPING".to_string());

                // Update shipping address city to uppercase
                order.shipping_address.city = order.shipping_address.city.to_uppercase();

                DataPayload::Order(order)
            }
            DataPayload::AddressUpdate(mut addr) => {
                // Normalize the address
                addr.street = addr.street.to_uppercase();
                addr.city = addr.city.to_uppercase();
                addr.state = addr.state.to_uppercase();
                addr.country = addr.country.to_uppercase();
                addr.postal_code = addr.postal_code.replace("-", "").replace(" ", "");

                DataPayload::AddressUpdate(addr)
            }
            DataPayload::SimpleMessage(msg) => {
                // Transform the message
                let processed = format!("[PROCESSED] {}", msg.to_uppercase());
                DataPayload::SimpleMessage(processed)
            }
            DataPayload::NumericData(mut nums) => {
                // Double all numbers and sort
                nums = nums.iter().map(|n| n * 2).collect();
                nums.sort();
                DataPayload::NumericData(nums)
            }
            DataPayload::Payment(payment) => {
                // Transform payment methods
                let transformed = match payment {
                    PaymentMethod::CreditCard(card) => {
                        // Mask the card number (keep last 4 digits)
                        let masked = if card.len() > 4 {
                            format!("****{}", &card[card.len() - 4..])
                        } else {
                            "****".to_string()
                        };
                        PaymentMethod::CreditCard(masked)
                    }
                    PaymentMethod::Paypal(email) => {
                        // Convert email to lowercase
                        PaymentMethod::Paypal(email.to_lowercase())
                    }
                    PaymentMethod::BankTransfer => PaymentMethod::BankTransfer,
                    PaymentMethod::Cash => PaymentMethod::Cash,
                };
                DataPayload::Payment(transformed)
            }
        }
    }
}

fn main() -> Result<()> {
    // Test with an order
    let input = DataPayload::Order(OrderInfo {
        order_id: "ORD-12345".to_string(),
        total_amount: 99.99,
        items: vec![
            "Widget".to_string(),
            "Gadget".to_string(),
            "Doodad".to_string(),
        ],
        shipping_address: Address {
            street: "123 Main St".to_string(),
            city: "Springfield".to_string(),
            state: "IL".to_string(),
            country: "USA".to_string(),
            postal_code: "62701".to_string(),
        },
    });

    component_run::<_, RunTy, (DataPayload,), (String,)>(
        ComponentFmt::File("test-modules/components/variant_large.wasm"),
        |mut linker| crimp_tests::bin!(@add linker, MyWorld),
        RunMode::InstantiateAndCallOnce {
            name: "main",
            params: (input,),
        },
    )
}
