# Multipart benchmarks

The standalone benchmark uses the existing Tokio and futures dependencies. Run
an optimized build with build artifacts and temporary files on D:

```powershell
$env:CARGO_TARGET_DIR = 'D:/BuildCache/multra-bench/target'
$env:TEMP = 'D:/BuildCache/multra-bench/tmp'
$env:TMP = $env:TEMP
New-Item -ItemType Directory -Force -Path $env:TEMP | Out-Null
cargo metadata --no-deps --format-version 1
cargo bench --bench multipart
```

Confirm `target_directory` in metadata before building. Set
`MULTRA_BENCH_ITERATIONS` to change the default 100 iterations, or
`MULTRA_BENCH_CASE` to filter case names by substring. For example, set the latter
to `single-large-field` to measure aggregation without the fragmented cases.

Cases cover a 4 MiB field in one source chunk, 1,024 small fields in one chunk,
an 8 KiB header delivered one byte at a time, and 8 KiB of delimiter transport
padding delivered one byte at a time. Inputs and source chunks are prepared
outside the timed section. Every field is collected with `Field::bytes()`;
results pass through `std::hint::black_box`. The current-thread runtime polls an
always-ready in-memory source, so these measurements exclude network and disk I/O.

Output reports input throughput, time per complete request, and source polls per
request, including EOF polls when the parser needs them. Source polling uses an
atomic counter in all cases. Fragmented header and padding cases intentionally
exercise incremental parsing and should be interpreted separately from ordinary
upload throughput. Repeat runs using the same toolchain, inputs, dependencies,
and machine; compare relative results and source poll counts, not absolute times
across different machines.

## Local aggregation comparison

On 2026-10-28, Windows x86_64 and Rust 1.95.0 release builds were compared using
this identical harness before and after the `Field::bytes()` aggregation change.
The baseline was commit `5155658`; both binaries used the same resolved
dependencies. Three runs of 200 iterations per case alternated baseline and
optimized binaries. Median times in microseconds per request were:

| Case | Before | After | Source polls, both versions |
| --- | ---: | ---: | ---: |
| Single large field | 1,868.93 | 1,016.19 | 1 |
| Many small fields | 501.12 | 458.08 | 1 |
| Fragmented header | 2,896.03 | 2,804.30 | 8,287 |
| Fragmented padding | 22,962.99 | 23,158.34 | 8,277 |

The large-field case spent about 46% less time per request in this local run.
The small-field measurements varied substantially; the fragmented cases show
no clear improvement from aggregation. These are in-memory parser measurements,
not predictions of application throughput. The single-chunk result can retain
the allocation containing other buffered fields; this avoids the aggregation
copy but does not guarantee lower retained memory for every application.
