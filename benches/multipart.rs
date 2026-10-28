use std::convert::Infallible;
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Poll;
use std::time::Instant;

use bytes::Bytes;
use futures_util::stream;
use multra::Multipart;

struct Case {
    name: &'static str,
    chunks: Vec<Bytes>,
    input_bytes: usize,
}

impl Case {
    fn new(name: &'static str, input: Vec<u8>, chunk_size: usize) -> Self {
        let input = Bytes::from(input);
        Self {
            name,
            chunks: (0..input.len())
                .step_by(chunk_size)
                .map(|start| input.slice(start..(start + chunk_size).min(input.len())))
                .collect(),
            input_bytes: input.len(),
        }
    }

    async fn parse(&self) -> usize {
        let polls = AtomicUsize::new(0);
        let mut chunks = self.chunks.iter();
        let stream = stream::poll_fn(|_cx| {
            polls.fetch_add(1, Ordering::Relaxed);
            Poll::Ready(chunks.next().cloned().map(Ok::<Bytes, Infallible>))
        });
        let mut multipart = Multipart::new(stream, "X-BOUNDARY");
        while let Some(field) = multipart
            .next_field()
            .await
            .expect("valid benchmark headers")
        {
            black_box(field.bytes().await.expect("valid benchmark field"));
        }
        polls.load(Ordering::Relaxed)
    }
}

fn cases() -> Vec<Case> {
    let opening = b"--X-BOUNDARY\r\nContent-Disposition: form-data; name=\"field\"\r\n";
    let closing = b"\r\n--X-BOUNDARY--\r\n";

    let mut large = opening.to_vec();
    large.extend_from_slice(b"\r\n");
    large.resize(large.len() + 4 * 1024 * 1024, b'x');
    large.extend_from_slice(closing);

    let mut small = Vec::new();
    for _ in 0..1024 {
        small.extend_from_slice(opening);
        small.extend_from_slice(b"\r\n0123456789abcdef0123456789abcdef\r\n");
    }
    small.extend_from_slice(b"--X-BOUNDARY--\r\n");

    let mut headers = opening.to_vec();
    headers.extend_from_slice(b"X-Long: ");
    headers.resize(headers.len() + 8 * 1024, b'a');
    headers.extend_from_slice(b"\r\n\r\nvalue");
    headers.extend_from_slice(closing);

    let mut padding = opening.to_vec();
    padding.extend_from_slice(b"\r\nvalue\r\n--X-BOUNDARY--");
    padding.resize(padding.len() + 8 * 1024, b' ');
    padding.extend_from_slice(b"\r\n");

    vec![
        Case::new("single-large-field", large, usize::MAX),
        Case::new("many-small-fields", small, usize::MAX),
        Case::new("fragmented-header", headers, 1),
        Case::new("fragmented-padding", padding, 1),
    ]
}

// Conversions only format benchmark counters, far below f64's exact integer limit.
#[allow(clippy::cast_precision_loss)]
fn main() {
    let iterations = std::env::var("MULTRA_BENCH_ITERATIONS").map_or(100, |value| {
        value.parse::<u32>().expect("positive iteration count")
    });
    assert!(iterations > 0, "iteration count must be positive");
    let filter = std::env::var("MULTRA_BENCH_CASE").unwrap_or_default();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("benchmark runtime");

    println!("case,iterations,input_bytes,MiB/s,us/request,source_polls/request");
    for case in cases()
        .into_iter()
        .filter(|case| case.name.contains(&filter))
    {
        runtime.block_on(case.parse());
        let started = Instant::now();
        let polls = runtime.block_on(async {
            let mut polls = 0;
            for _ in 0..iterations {
                polls += black_box(case.parse().await);
            }
            polls
        });
        let seconds = started.elapsed().as_secs_f64();
        println!(
            "{},{},{},{:.2},{:.2},{:.2}",
            case.name,
            iterations,
            case.input_bytes,
            case.input_bytes as f64 * f64::from(iterations) / seconds / (1024.0 * 1024.0),
            seconds * 1_000_000.0 / f64::from(iterations),
            polls as f64 / f64::from(iterations),
        );
    }
}
