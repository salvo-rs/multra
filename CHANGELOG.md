# Changelog

## 1.3.0 - 2026-11-17

### Compatibility and upgrade notes

- Quoted `Content-Disposition` names and filenames now decode all MIME
  quoted-pairs before filename percent decoding. A backslash escapes the next
  character: transmit literal backslashes as `\\` inside quoted parameters.
  Correctly escaped Windows paths remain supported; legacy quoted values with
  raw single backslashes change interpretation. Unquoted backslashes are unchanged.
- Finite `SizeLimit::whole_stream()` limits now include the epilogue after the
  closing boundary. Completing iteration waits for source EOF and propagates
  epilogue source errors. The default unbounded limit (`u64::MAX`) still stops
  at the closing boundary.
- Repeated part headers are preserved through `Field::headers().get_all()`.
  Field metadata still uses the last `Content-Disposition` and `Content-Type`
  values, including an invalid last value. Direct `HeaderMap::get()` or indexing
  now observes the first preserved value instead of the overwritten last value.
- `Field::content_type()` returns `None` when the complete parameter set cannot
  be represented faithfully by `mime::Mime`. Raw headers remain available, and
  text decoding independently honors the parsed charset.
- Malformed Content-Type syntax can return the new `Error::InvalidContentType`
  variant. The minimum supported Rust version remains 1.95.

### Parsing and correctness

- Decode quoted field names before applying field-name constraints, and avoid
  unescaping bytes introduced by filename percent decoding. (#34)
- Require exactly two hexadecimal digits in percent escapes. Preserve the
  unquoted original filename when escapes are malformed or produce invalid
  UTF-8. (#29)
- Decode extended filenames without BOM-driven charset switching, and interpret
  declared `ISO-8859-1` using its exact byte mapping. Invalid extended values
  retain the ordinary filename fallback. (#36)
- Parse Content-Type parameters with HTTP whitespace, empty parameter segments,
  and quoted-pair rules. Preserve opaque quoted parameter bytes without losing
  an independently valid charset. (#41)
- Reject other media types such as `multipart/form-data+json`. (#30)
- Reject inconsistent header termination instead of silently discarding a
  field-body prefix. (#31)
- Preserve repeated headers while keeping the existing last-value metadata
  selection policy. (#35)
- Enforce finite whole-stream limits consistently across epilogues, independently
  of source chunk boundaries. (#37)

### Memory and scheduling

- Avoid copying entire upstream `Bytes` chunks before stage limits are checked.
  Coalesce bounded prefixes for fragmented headers and delimiters. (#40)
- Yield during field aggregation after every 32 field chunks, improving executor
  cooperation for `bytes()`, `text()`, and `json()`. (#38)
- Avoid unnecessary self-wakes when reads return Ready, while retaining wakeups
  needed for incomplete input and cooperative yielding. (#39)

### Examples and maintenance

- Return HTTP 400 for malformed uploads and HTTP 413 for configured resource-limit
  failures in the Hyper example. Internal failures remain HTTP 500. (#32)
- Remove an accidentally tracked local worktree reference and ignore local
  `.claude/worktrees/` directories. (#33)
- Add regressions for parameter decoding, repeated headers, epilogues, allocation
  behavior, scheduling, and fragmented input.

Full changes: https://github.com/salvo-rs/multra/compare/v1.2.0...v1.3.0

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
