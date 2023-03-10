# DEQS Test CLI

A test cli for the decentralized quoting service.

## Usage and Documentation

1. Start the server with:

    MC_LOG=trace IAS_MODE=DEV SGX_MODE=SW cargo run --bin deqs-server -- --client-listen-uri insecure-deqs://127.0.0.1/ --db-path {sqllite_path} --ledger-db {ledgerdb_path}

2. Submit SCIs using:

    MC_LOG=trace IAS_MODE=DEV SGX_MODE=SW cargo run --bin deqs-test-cli -- --rng-seed 1 submit-quotes --num-quotes 3 --deqs-uri insecure-deqs://127.0.0.1/ --ledger-db-path {ledgerdb_path}

3. Test liveupdates via grpcurl:

    ./build-protoset
    grpcurl -d '' -vv -plaintext -protoset ./deqs.protoset 127.0.0.1:7000 deqs.DeqsClientAPI.LiveUpdates
