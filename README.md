# DEQS

A decentralized quoting service.

The quoting service is capable of storing and distributing "quotes", and tracking
when they expire.

Here a "quote" means an [MCIP #31](https://github.com/mobilecoinfoundation/mcips/31) "Signed Contingent Input",
which is (informally) a signed offer to trade one cryptocurrency on the Mobilecoin network for another at some price.

See also [MCIP #42](https://github.com/mobilecoinfoundation/mcips/42) which added support for partial fills.

This quoting service can be used to facilitate private peer-to-peer trades.

## Overview

The DEQS is organized as a decentralized peer-to-peer network for distributing signed quotes.
Any MobileCoin network participant is able to take one or more signed quotes, incorporate them into a transaction,
and settle that transaction to the MobileCoin blockchain, completing a trade.

This does not reveal the identity of any party to the other, or reveal the volume of the trade, if partial fills
are being used. Because a Quote reveals the key image of the input that it signed over, and reveals the public keys of
any required TxOut's, it becomes clear to anyone watching the DEQS that this quote was filled.

Filling quotes in this way is inherently racy, if two people try to fill a quote at the same time, only one of them
can succeed, because the mobilecoin network prevents the same key image from appearing twice in the blockchain in order
to prevent double-spends generally.

The DEQS node offers several features.

The DeqsClient GRPC API:

1. Submit quotes
1. Get quotes of a desired type, from the node's local quote book
1. Subscribe to streaming updates of the available quotes

A gossip network protocol based on libp2p (internal to the server):

1. Broadcast a signed quote to peers
1. Request all quotes from peers

A local sqllite database that backs up the known quotes:

1. Helps the node recover quickly if it is rebooted.

The DEQS node relies on a local copy of the mobilecoin ledger, which it uses to validate quotes
and prune expired quotes. This needs to be synced by a separate process, such as `mobilecoind` or `full-service`.

## Usage and Documentation

TODO

## Build

1. Install Rust from [https://www.rust-lang.org/tools/install](https://www.rust-lang.org/tools/install)

2. Install dependencies.

   On Ubuntu:

    ```sh
    sudo apt install build-essential protobuf-compiler
    ```

3. Pull submodule.

    ```sh
    git submodule update --init --recursive
    ```

4. Run cargo as usual

    ```sh
    cargo build
    ```

## License

This code is available under open-source licenses. Look for the [LICENSE](./LICENSE) file for more
information.
