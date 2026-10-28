# Fuzzing

Install nightly Rust and `cargo-fuzz`:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz --locked
```

Run any available target where `$target` is the name of the target and `$n` is
the number of CPUs to use for fuzzing:

```sh
cargo +nightly fuzz list # get list of targets
cargo +nightly fuzz run $target -j $n
```

For a bounded smoke run against the checked-in corpus:

```sh
cargo +nightly fuzz run fuzz_multipart_bytes fuzz/corpus/fuzz_multipart_bytes -- -max_total_time=60 -timeout=5 -max_len=16384
```

CI builds the target separately, then runs this command with a two-minute step
deadline. `-timeout=5` detects a single input stuck inside a parser poll; the
target's poll budget detects repeated polls that make no progress. Neither relies
on an async timeout interrupting a synchronous loop.

The target covers complete bodies, bytewise and other chunk widths, and rotates
between reading `Field::chunk()`, collecting `Field::bytes()`, and dropping unread
fields. Total body, field, preamble, and header sizes are bounded. It uses manual
polling over a finite, always-ready source, without creating a runtime per input.
Real pending I/O and cancellation/resumption are covered by `tests/streaming.rs`.

Existing corpus files remain raw multipart bodies with boundary `X-BOUNDARY`.
Inputs starting with the bytes `00 4d` use an optional envelope:

```text
00 4d | boundary length (u8) | boundary bytes | control (u8) | multipart body
```

The control byte selects chunk widths with its low two bits (whole body, 1, 2,
or 17 bytes) and the initial field-reading mode with its remaining bits.
Boundary bytes are validated through `Multipart::try_with_constraints`, so
invalid boundaries also exercise constructor rejection. The additional envelope
seed exercises a different valid boundary and all three field-reading modes.

On Windows, keep build and temporary files on D: as required by the repository's
local build policy. Set `CARGO_TARGET_DIR`, `TEMP`, and `TMP` to directories under
`D:/BuildCache/multra-fuzz` and verify `cargo metadata --no-deps
--format-version 1` reports that target directory before compiling. Sanitizer
fuzzing normally runs on Linux; the CI job provides the bounded smoke check.
