// @generated automatically by Diesel CLI.

diesel::table! {
    use diesel::sql_types::*;
    use crate::sql_types::*;

    quotes (id) {
        id -> Binary,
        sci -> Binary,
        base_token_id -> BigInt,
        counter_token_id -> BigInt,
        base_range_min -> Binary,
        base_range_max -> Binary,
        max_counter_amount -> Binary,
        timestamp -> BigInt,
    }
}
