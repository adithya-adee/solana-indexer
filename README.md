# Solana Indexer

This repository contains the `solana-indexer-sdk` and a set of examples and benchmarks.

## `solana-indexer-sdk`

The `solana-indexer-sdk` is a lightweight, customizable, and high-performance SDK for indexing data from the Solana blockchain.
Internally, the SDK's core architecture is cleanly separated into modular sub-components (`execution`, `registry`, `backfill`, and `decoding`), making it robust and easy to contribute to.

For more information, please see the [SDK's README](solana-indexer-sdk/README.md).

## Examples

The `examples` directory contains a set of examples that demonstrate how to use the SDK to build custom indexers. This includes an OpenTelemetry (`observability/`) stack featuring Docker-based local setup of Grafana, Prometheus, and Tempo.

For more information, please see the [examples' README](examples/README.md).

## Benches

The `benches` directory contains a set of benchmarks for the SDK.

For more information, please see the [benches' README](benches/README.md).

## Contributing

Contributions are welcome! Please see the [contributing guide](CONTRIBUTING.md) for more information.

## License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.
