// Copyright (c) 2023 MobileCoin Inc.

mod common;

use common::{create_partial_sci, create_sci, pair};
use deqs_quote_book::Quote;
use rand::{prelude::SliceRandom, rngs::StdRng, SeedableRng};

#[test]
fn test_max_tokens() {
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Test max tokens for the non-partial-fill scenario
    let sci = create_sci(&pair(), 10, 20, &mut rng);
    let quote = Quote::try_from(sci).unwrap();

    assert_eq!(quote.max_base_tokens(), 10);
    assert_eq!(quote.max_counter_tokens(), 20);

    // Test max tokens for a partial fill with no change and no minimum.
    let sci = create_partial_sci(&pair(), 10, 0, 0, 100, &mut rng);
    let quote = Quote::try_from(sci).unwrap();

    assert_eq!(quote.max_base_tokens(), 10);
    assert_eq!(quote.max_counter_tokens(), 100);

    // Test max tokens for a partial fill with no change and a minimum.
    let sci = create_partial_sci(&pair(), 10, 7, 0, 100, &mut rng);
    let quote = Quote::try_from(sci).unwrap();

    assert_eq!(quote.max_base_tokens(), 10);
    assert_eq!(quote.max_counter_tokens(), 100);

    // Test max tokens for a partial fill with change and no minimum.
    let sci = create_partial_sci(&pair(), 10, 0, 5, 100, &mut rng);
    let quote = Quote::try_from(sci).unwrap();

    assert_eq!(quote.max_base_tokens(), 5);
    assert_eq!(quote.max_counter_tokens(), 100);

    // Test max tokens for a partial fill with change and a minimum.
    let sci = create_partial_sci(&pair(), 10, 3, 5, 100, &mut rng);
    let quote = Quote::try_from(sci).unwrap();

    assert_eq!(quote.max_base_tokens(), 5);
    assert_eq!(quote.max_counter_tokens(), 100);
}

#[test]
fn test_sorting() {
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    let quote_1_for_10 = Quote::try_from(create_sci(&pair(), 1, 10, &mut rng)).unwrap();
    let quote_2_for_10 = Quote::try_from(create_sci(&pair(), 2, 10, &mut rng)).unwrap();
    let quote_3_for_10 = Quote::try_from(create_sci(&pair(), 3, 10, &mut rng)).unwrap();
    let quote_1_for_5 = Quote::try_from(create_sci(&pair(), 1, 5, &mut rng)).unwrap();
    let quote_2_for_5 = Quote::try_from(create_sci(&pair(), 2, 5, &mut rng)).unwrap();
    let quote_3_for_5 = Quote::try_from(create_sci(&pair(), 3, 5, &mut rng)).unwrap();
    let quote_10_for_10 = Quote::try_from(create_sci(&pair(), 10, 10, &mut rng)).unwrap();
    let quote_20_for_10 = Quote::try_from(create_sci(&pair(), 20, 10, &mut rng)).unwrap();
    let quote_30_for_10 = Quote::try_from(create_sci(&pair(), 30, 10, &mut rng)).unwrap();

    let all_quotes = vec![
        &quote_1_for_10,
        &quote_2_for_10,
        &quote_3_for_10,
        &quote_1_for_5,
        &quote_2_for_5,
        &quote_3_for_5,
        &quote_10_for_10,
        &quote_20_for_10,
        &quote_30_for_10,
    ];

    let expected_quotes = vec![
        &quote_30_for_10, // 30/10 = 3
        &quote_20_for_10, // 20/10 = 2
        &quote_10_for_10, // 10/10 = 1
        &quote_3_for_5,   // 3/5 = 0.6
        &quote_2_for_5,   // 2/5 = 0.4
        &quote_3_for_10,  // 3/10 = 0.3
        &quote_2_for_10,  // 2/10 = 0.2 (was created before the next one)
        &quote_1_for_5,   // 1/5 = 0.2
        &quote_1_for_10,  // 1/10 = 0.1
    ];

    let mut quotes = all_quotes.clone();
    quotes.sort();
    assert_eq!(quotes, expected_quotes);

    let mut quotes = all_quotes.clone();
    quotes.reverse();
    quotes.sort();
    assert_eq!(quotes, expected_quotes);

    let mut quotes = all_quotes.clone();
    quotes.shuffle(&mut rng);
    quotes.sort();
    assert_eq!(quotes, expected_quotes);
}

#[test]
fn counter_tokens_cost_works_for_non_partial_fill_scis() {
    let pair = pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Adding an quote should work
    let sci = create_sci(&pair, 10, 20, &mut rng);
    let quote = Quote::try_from(sci).unwrap();

    // We can only calculate cost for the exact amount of base tokens since this is
    // not a partial fill.
    assert_eq!(quote.counter_tokens_cost(10), Ok(20));
    assert!(quote.counter_tokens_cost(9).is_err());
    assert!(quote.counter_tokens_cost(11).is_err());
    assert!(quote.counter_tokens_cost(0).is_err());
    assert!(quote.counter_tokens_cost(u64::MAX).is_err());
}

#[test]
fn counter_tokens_cost_works_for_partial_fill_no_change_no_min() {
    let pair = pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Trading at a ratio of 1 base token to 10 counter tokens
    let sci = create_partial_sci(&pair, 10, 0, 0, 100, &mut rng);
    let quote = Quote::try_from(sci).unwrap();
    assert_eq!(quote.counter_tokens_cost(10), Ok(100));
    assert_eq!(quote.counter_tokens_cost(5), Ok(50));
    assert_eq!(quote.counter_tokens_cost(0), Ok(0));

    assert!(quote.counter_tokens_cost(11).is_err());
    assert!(quote.counter_tokens_cost(u64::MAX).is_err());

    // Trading at a ratio of 10 base token to 1 counter tokens
    let sci = create_partial_sci(&pair, 100, 0, 0, 10, &mut rng);
    let quote = Quote::try_from(sci).unwrap();
    assert_eq!(quote.counter_tokens_cost(100), Ok(10));
    assert_eq!(quote.counter_tokens_cost(50), Ok(5));
    assert_eq!(quote.counter_tokens_cost(51), Ok(5));
    assert_eq!(quote.counter_tokens_cost(59), Ok(5));
    assert_eq!(quote.counter_tokens_cost(60), Ok(6));
    assert_eq!(quote.counter_tokens_cost(1), Ok(0)); // rounding down, 1 token is not enough to get any counter tokens
    assert_eq!(quote.counter_tokens_cost(0), Ok(0));

    assert!(quote.counter_tokens_cost(101).is_err());
    assert!(quote.counter_tokens_cost(u64::MAX).is_err());
}

#[test]
fn counter_tokens_cost_works_for_partial_fill_no_change_with_min() {
    let pair = pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Trading at a ratio of 1 base token to 10 counter tokens
    let sci = create_partial_sci(&pair, 10, 7, 0, 100, &mut rng);
    let quote = Quote::try_from(sci).unwrap();
    assert_eq!(quote.counter_tokens_cost(10), Ok(100));
    assert_eq!(quote.counter_tokens_cost(7), Ok(70));

    assert!(quote.counter_tokens_cost(6).is_err()); // below the min fill amount
    assert!(quote.counter_tokens_cost(0).is_err()); // below the min fill amount
    assert!(quote.counter_tokens_cost(11).is_err()); // above the max amount offered
    assert!(quote.counter_tokens_cost(u64::MAX).is_err()); // above the max amount offered

    // Trading at a ratio of 10 base token to 1 counter tokens
    let sci = create_partial_sci(&pair, 100, 55, 0, 10, &mut rng);
    let quote = Quote::try_from(sci).unwrap();
    assert_eq!(quote.counter_tokens_cost(100), Ok(10));
    assert_eq!(quote.counter_tokens_cost(55), Ok(5)); // rounding down
    assert_eq!(quote.counter_tokens_cost(59), Ok(5)); // rounding down
    assert_eq!(quote.counter_tokens_cost(60), Ok(6));

    assert!(quote.counter_tokens_cost(0).is_err()); // below the min fill amount
    assert!(quote.counter_tokens_cost(1).is_err()); // below the min fill amount
    assert!(quote.counter_tokens_cost(54).is_err()); // below the min fill amount
    assert!(quote.counter_tokens_cost(101).is_err()); // above the max amount offered
    assert!(quote.counter_tokens_cost(u64::MAX).is_err()); // above the max
                                                           // amount offered
}

#[test]
fn counter_tokens_cost_works_for_partial_fill_with_change_no_min() {
    let pair = pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Trading at a ratio of 1 base token to 10 counter tokens
    let sci = create_partial_sci(&pair, 10, 0, 3, 100, &mut rng);
    let quote = Quote::try_from(sci).unwrap();
    assert_eq!(quote.counter_tokens_cost(7), Ok(70));
    assert_eq!(quote.counter_tokens_cost(6), Ok(60));
    assert_eq!(quote.counter_tokens_cost(1), Ok(10));
    assert_eq!(quote.counter_tokens_cost(0), Ok(0));

    assert!(quote.counter_tokens_cost(8).is_err()); // we need to be able to pay 3 out of the 10 back, 8 will only leave out 2
    assert!(quote.counter_tokens_cost(u64::MAX).is_err());

    // Trading at a ratio of 10 base token to 1 counter tokens
    let sci = create_partial_sci(&pair, 100, 0, 30, 10, &mut rng);
    let quote = Quote::try_from(sci).unwrap();
    assert_eq!(quote.counter_tokens_cost(70), Ok(7));
    assert_eq!(quote.counter_tokens_cost(60), Ok(6));
    assert_eq!(quote.counter_tokens_cost(61), Ok(6));
    assert_eq!(quote.counter_tokens_cost(69), Ok(6));
    assert_eq!(quote.counter_tokens_cost(1), Ok(0)); // rounding down, 1 token is not enough to get any counter tokens
    assert_eq!(quote.counter_tokens_cost(0), Ok(0));

    assert!(quote.counter_tokens_cost(71).is_err()); // exceeds max available (since we require a change of 30 this allows for up to
                                                     // 70 to be swapped)
    assert!(quote.counter_tokens_cost(101).is_err()); // exceeds max available
    assert!(quote.counter_tokens_cost(u64::MAX).is_err());
}

#[test]
fn counter_tokens_cost_works_for_partial_fill_with_change_and_min() {
    let pair = pair();
    let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

    // Trading at a ratio of 1 base token to 10 counter tokens
    // Allowing a trade of between 5 and 7 tokens (since min_base_fill_amount is 5
    // and required change is 3, leaving 7)
    let sci = create_partial_sci(&pair, 10, 5, 3, 100, &mut rng);
    let quote = Quote::try_from(sci).unwrap();
    assert_eq!(quote.counter_tokens_cost(7), Ok(70));
    assert_eq!(quote.counter_tokens_cost(6), Ok(60));
    assert_eq!(quote.counter_tokens_cost(5), Ok(50));

    assert!(quote.counter_tokens_cost(8).is_err()); // we need to be able to pay 3 out of the 10 back, 8 will only leave out 2
    assert!(quote.counter_tokens_cost(4).is_err()); // below the minimum of 5 required
    assert!(quote.counter_tokens_cost(u64::MAX).is_err());

    // Trading at a ratio of 10 base token to 1 counter tokens
    // Allowing a trade between 50 and 70 tokens (since min_base_fill_amount is 50,
    // and required change is 30, leaving up to 70)
    let sci = create_partial_sci(&pair, 100, 50, 30, 10, &mut rng);
    let quote = Quote::try_from(sci).unwrap();
    assert_eq!(quote.counter_tokens_cost(70), Ok(7));
    assert_eq!(quote.counter_tokens_cost(50), Ok(5));
    assert_eq!(quote.counter_tokens_cost(51), Ok(5));
    assert_eq!(quote.counter_tokens_cost(59), Ok(5));
    assert!(quote.counter_tokens_cost(71).is_err()); // exceeds max available (since we require a change of 30 this allows for up to
                                                     // 70 to be swapped)
    assert!(quote.counter_tokens_cost(101).is_err()); // exceeds max available
    assert!(quote.counter_tokens_cost(u64::MAX).is_err());
    assert!(quote.counter_tokens_cost(49).is_err()); // below min_partial_fill_value
    assert!(quote.counter_tokens_cost(1).is_err()); // below min_partial_fill_value
    assert!(quote.counter_tokens_cost(0).is_err()); // below min_partial_fill_value
}
