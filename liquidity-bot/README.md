# DEQS Liquidity Bot

The liquidity bot is a DEQS bot whose purpose is to provide liquidity on a DEQS server by submitting SCIs for TxOuts it owns.
The current implemnentation is geared towards the most simple scenario: each invocation of the bot submits SCIs that are offering a base token in return for a counter token at a hardcoded ratio.

The current implementation operates on a single account, configured via an account key file, and relies on a local ledger that is provided by full-service or mobilecoind (or anything else that synchronizes a ledger).

The current implementation does its own view-key scanning and stores state inside a wallet file. This might get replaced by using full-service in the future, but was done this way initially for the sake of making it easier to run and test locally.

## Usage

If we want to have the bot offer MOB in exchange for eUSD at the rate of 2 eUSD per MOB, we could start the bot like this:

```
    cargo run --bin deqs-liquidity-bot -- \
        --deqs insecure-deqs://localhost \
        --base-token-id 0 \
        --counter-token-id 1 \
        --swap-rate 2 \
        --ledger-db /tmp/testnet.ledger \
        --account-key key.json \
        --wallet-db wallet.db \
```

The bot will then scan the ledger (you can specify a start block with `--first-block-index ...` if you know the first block where the bot is expected to find transactions). When it encounters a TxOut that belongs to it, and is for the base token id, it will try and submit it to the DEQS. It will also keep track of it, repeatedly trying to resubmit until it becomes stale. This helps in case the DEQS server restarts and loses its quotebook.
