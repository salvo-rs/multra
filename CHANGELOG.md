# Changelog

## 1.2.0 - 2026-10-29

### Compatibility

- The minimum supported Rust version is now 1.95.
- Boundary transport padding is limited to 8 KiB per delimiter by default.
  Applications requiring larger MIME padding can configure
  `SizeLimit::boundary_padding()`.
- Opening boundaries must start a line. Closing boundaries now reject invalid
  suffixes and require a complete line ending or end of stream.

### Security and correctness

- Parse complete quoted `Content-Disposition` parameters before extracting
  `name`, `filename`, and `filename*`, preventing values from changing field
  identity or constraint selection.
- Bound delimiter padding independently of field size and scan it incrementally.
- Limit empty chunks processed in a single poll so an always-ready source yields
  to the executor.
- Add optional `Constraints::max_fields()` and `Error::FieldCountExceeded` to
  reject requests containing too many fields, including empty or repeated fields.
- Require Tokio 1.53.3 or newer and audit library and fuzz dependencies in CI.

### Performance

- Consume buffered fields before reading more source chunks to preserve
  backpressure.
- Search fragmented headers and preambles incrementally.
- Return single-chunk field bodies without an aggregation copy. Returned bytes
  can share an allocation with other buffered data.
- Add repeatable throughput and source-poll benchmarks.

### Testing and documentation

- Expand fuzz coverage across boundaries, chunk sizes, size limits, and field
  consumption modes; run a bounded ASan fuzz smoke test in CI.
- Cover actual pending I/O, cancellation and resumption, and chunk-split
  invariance in integration tests.
- Clarify filename trust boundaries, parser recovery expectations, and constrained
  reader examples.

Full changes: https://github.com/salvo-rs/multra/compare/v1.1.0...v1.2.0
