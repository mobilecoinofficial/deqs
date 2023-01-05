DEQS
====

A decentralized quoting service.

The quoting service is capable of storing and distributing "quotes", and tracking
when they expire.

Here a "quote" means an [MCIP #31](https://github.com/mobilecoinfoundation/mcips/31) "Signed Contingent Input",
which is (informally) a signed offer to trade one cryptocurrency on the Mobilecoin network for another at some price.

This quoting service can be used to facilitate private peer-to-peer trades.

### License

This code is available under open-source licenses. Look for the [LICENSE](./LICENSE) file for more
information.

### Usage and Documentation

TODO

### Build

1. Install Rust from https://www.rust-lang.org/tools/install

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
