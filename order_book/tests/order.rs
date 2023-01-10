// Copyright (c) 2023 MobileCoin Inc.

mod common;

use common::{create_partial_sci, create_sci, pair};
use deqs_order_book::Order;
use rand::{rngs::StdRng, SeedableRng};

#[test]
fn counter_tokens_cost_works_for_non_partial_fill_scis() {
    let pair = pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Adding an order should work
    let sci = create_sci(&pair, 10, 20, &mut rng);
    let order = Order::try_from(sci).unwrap();

    // We can only calculate cost for the exact amount of base tokens since this is
    // not a partial fill.
    assert_eq!(order.counter_tokens_cost(10), Ok(20));
    assert!(order.counter_tokens_cost(9).is_err());
    assert!(order.counter_tokens_cost(11).is_err());
    assert!(order.counter_tokens_cost(0).is_err());
    assert!(order.counter_tokens_cost(u64::MAX).is_err());
}

#[test]
fn counter_tokens_cost_works_for_partial_fill_no_change_no_min() {
    let pair = pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Trading at a ratio of 1 base token to 10 counter tokens
    let sci = create_partial_sci(&pair, 10, 0, 0, 100, &mut rng);
    let order = Order::try_from(sci).unwrap();
    assert_eq!(order.counter_tokens_cost(10), Ok(100));
    assert_eq!(order.counter_tokens_cost(5), Ok(50));
    assert_eq!(order.counter_tokens_cost(0), Ok(0));

    assert!(order.counter_tokens_cost(11).is_err());
    assert!(order.counter_tokens_cost(u64::MAX).is_err());

    // Trading at a ratio of 10 base token to 1 counter tokens
    let sci = create_partial_sci(&pair, 100, 0, 0, 10, &mut rng);
    let order = Order::try_from(sci).unwrap();
    assert_eq!(order.counter_tokens_cost(100), Ok(10));
    assert_eq!(order.counter_tokens_cost(50), Ok(5));
    assert_eq!(order.counter_tokens_cost(51), Ok(5));
    assert_eq!(order.counter_tokens_cost(59), Ok(5));
    assert_eq!(order.counter_tokens_cost(60), Ok(6));
    assert_eq!(order.counter_tokens_cost(1), Ok(0)); // rounding down, 1 token is not enough to get any counter tokens
    assert_eq!(order.counter_tokens_cost(0), Ok(0));

    assert!(order.counter_tokens_cost(101).is_err());
    assert!(order.counter_tokens_cost(u64::MAX).is_err());
}

#[test]
fn counter_tokens_cost_works_for_partial_fill_no_change_with_min() {
    let pair = pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Trading at a ratio of 1 base token to 10 counter tokens
    let sci = create_partial_sci(&pair, 10, 7, 0, 100, &mut rng);
    let order = Order::try_from(sci).unwrap();
    assert_eq!(order.counter_tokens_cost(10), Ok(100));
    assert_eq!(order.counter_tokens_cost(7), Ok(70));

    assert!(order.counter_tokens_cost(6).is_err()); // below the min fill amount
    assert!(order.counter_tokens_cost(0).is_err()); // below the min fill amount
    assert!(order.counter_tokens_cost(11).is_err()); // above the max amount offered
    assert!(order.counter_tokens_cost(u64::MAX).is_err()); // above the max amount offered

    // Trading at a ratio of 10 base token to 1 counter tokens
    let sci = create_partial_sci(&pair, 100, 55, 0, 10, &mut rng);
    let order = Order::try_from(sci).unwrap();
    assert_eq!(order.counter_tokens_cost(100), Ok(10));
    assert_eq!(order.counter_tokens_cost(55), Ok(5)); // rounding down
    assert_eq!(order.counter_tokens_cost(59), Ok(5)); // rounding down
    assert_eq!(order.counter_tokens_cost(60), Ok(6));

    assert!(order.counter_tokens_cost(0).is_err()); // below the min fill amount
    assert!(order.counter_tokens_cost(1).is_err()); // below the min fill amount
    assert!(order.counter_tokens_cost(54).is_err()); // below the min fill amount
    assert!(order.counter_tokens_cost(101).is_err()); // above the max amount offered
    assert!(order.counter_tokens_cost(u64::MAX).is_err()); // above the max
                                                           // amount offered
}

#[test]
fn counter_tokens_cost_works_for_partial_fill_with_change_no_min() {
    let pair = pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Trading at a ratio of 1 base token to 10 counter tokens
    let sci = create_partial_sci(&pair, 10, 0, 3, 100, &mut rng);
    let order = Order::try_from(sci).unwrap();
    assert_eq!(order.counter_tokens_cost(7), Ok(70));
    assert_eq!(order.counter_tokens_cost(6), Ok(60));
    assert_eq!(order.counter_tokens_cost(1), Ok(10));
    assert_eq!(order.counter_tokens_cost(0), Ok(0));

    assert!(order.counter_tokens_cost(8).is_err()); // we need to be able to pay 3 out of the 10 back, 8 will only leave out 2
    assert!(order.counter_tokens_cost(u64::MAX).is_err());

    // Trading at a ratio of 10 base token to 1 counter tokens
    let sci = create_partial_sci(&pair, 100, 0, 30, 10, &mut rng);
    let order = Order::try_from(sci).unwrap();
    assert_eq!(order.counter_tokens_cost(70), Ok(7));
    assert_eq!(order.counter_tokens_cost(60), Ok(6));
    assert_eq!(order.counter_tokens_cost(61), Ok(6));
    assert_eq!(order.counter_tokens_cost(69), Ok(6));
    assert_eq!(order.counter_tokens_cost(1), Ok(0)); // rounding down, 1 token is not enough to get any counter tokens
    assert_eq!(order.counter_tokens_cost(0), Ok(0));

    assert!(order.counter_tokens_cost(71).is_err()); // exceeds max available (since we require a change of 30 this allows for up to
                                                     // 70 to be swapped)
    assert!(order.counter_tokens_cost(101).is_err()); // exceeds max available
    assert!(order.counter_tokens_cost(u64::MAX).is_err());
}

#[test]
fn counter_tokens_cost_works_for_partial_fill_with_change_and_min() {
    let pair = pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Trading at a ratio of 1 base token to 10 counter tokens
    // Allowing a trade of between 5 and 7 tokens (since min_base_fill_amount is 5
    // and required change is 3, leaving 7)
    let sci = create_partial_sci(&pair, 10, 5, 3, 100, &mut rng);
    let order = Order::try_from(sci).unwrap();
    assert_eq!(order.counter_tokens_cost(7), Ok(70));
    assert_eq!(order.counter_tokens_cost(6), Ok(60));
    assert_eq!(order.counter_tokens_cost(5), Ok(50));

    assert!(order.counter_tokens_cost(8).is_err()); // we need to be able to pay 3 out of the 10 back, 8 will only leave out 2
    assert!(order.counter_tokens_cost(4).is_err()); // below the minimum of 5 required
    assert!(order.counter_tokens_cost(u64::MAX).is_err());

    // Trading at a ratio of 10 base token to 1 counter tokens
    // Allowing a trade between 50 and 70 tokens (since min_base_fill_amount is 50,
    // and required change is 30, leaving up to 70)
    let sci = create_partial_sci(&pair, 100, 50, 30, 10, &mut rng);
    let order = Order::try_from(sci).unwrap();
    assert_eq!(order.counter_tokens_cost(70), Ok(7));
    assert_eq!(order.counter_tokens_cost(50), Ok(5));
    assert_eq!(order.counter_tokens_cost(51), Ok(5));
    assert_eq!(order.counter_tokens_cost(59), Ok(5));
    assert!(order.counter_tokens_cost(71).is_err()); // exceeds max available (since we require a change of 30 this allows for up to
                                                     // 70 to be swapped)
    assert!(order.counter_tokens_cost(101).is_err()); // exceeds max available
    assert!(order.counter_tokens_cost(u64::MAX).is_err());
    assert!(order.counter_tokens_cost(49).is_err()); // below min_partial_fill_value
    assert!(order.counter_tokens_cost(1).is_err()); // below min_partial_fill_value
    assert!(order.counter_tokens_cost(0).is_err()); // below min_partial_fill_value
}
