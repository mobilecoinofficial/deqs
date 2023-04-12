// Copyright (c) 2018-2022 The MobileCoin Foundation

//! DEQS gRPC API.

mod autogenerated_code {
    pub use deqs_api::deqs;
    pub use mc_api::external;
    pub use protobuf::well_known_types::Empty;

    // Needed due to how to the auto-generated code references the Empty message.
    pub mod empty {
        pub use protobuf::well_known_types::Empty;
    }

    // Include the auto-generated code.
    include!(concat!(env!("OUT_DIR"), "/protos-auto-gen/mod.rs"));
}

pub use crate::autogenerated_code::*;

pub use mc_api::ConversionError;