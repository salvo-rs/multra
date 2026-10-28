#![no_main]

use std::convert::Infallible;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use futures_util::stream;
use libfuzzer_sys::fuzz_target;
use multra::bytes::Bytes;
use multra::{Constraints, Multipart, SizeLimit};

fn drive<F: Future>(future: F, budget: &mut usize) -> F::Output {
    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        assert!(*budget > 0, "parser exceeded poll budget");
        *budget -= 1;
        if let Poll::Ready(result) = future.as_mut().poll(&mut cx) {
            return result;
        }
    }
}

fuzz_target!(|data: &[u8]| {
    // Existing corpus entries remain raw bodies with the original boundary.
    // The optional envelope is: NUL M, boundary length, boundary, control, body.
    let (boundary, control, body) = if data.starts_with(b"\0M") && data.len() >= 4 {
        let len = usize::from(data[2]).min(data.len() - 4);
        (
            String::from_utf8_lossy(&data[3..3 + len]),
            data[3 + len],
            &data[4 + len..],
        )
    } else {
        (
            "X-BOUNDARY".into(),
            data.first().copied().unwrap_or(0),
            data,
        )
    };
    let chunk_size = match control & 3 {
        0 => body.len().max(1),
        1 => 1,
        2 => 2,
        _ => 17,
    };
    let chunks = body
        .chunks(chunk_size)
        .map(|chunk| Ok::<_, Infallible>(Bytes::copy_from_slice(chunk)));
    let constraints = Constraints::new().size_limit(
        SizeLimit::new()
            .whole_stream(12 * 1024)
            .per_field(4 * 1024)
            .preamble(1024)
            .headers(2 * 1024),
    );
    let Ok(mut multipart) =
        Multipart::try_with_constraints(stream::iter(chunks), boundary, constraints)
    else {
        return;
    };

    // The finite, always-ready source needs no runtime or I/O wakeups. A poll
    // budget detects a parser that stops making progress; libFuzzer's timeout
    // separately catches loops inside a single poll.
    let mut budget = body.len().saturating_mul(4).saturating_add(128);
    let mut mode = control >> 2;
    while let Ok(Some(mut field)) = drive(multipart.next_field(), &mut budget) {
        match mode % 3 {
            0 => drop(field),
            1 => loop {
                match drive(field.chunk(), &mut budget) {
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(_) => return,
                }
            },
            _ => {
                if drive(field.bytes(), &mut budget).is_err() {
                    return;
                }
            }
        }
        mode = mode.wrapping_add(1);
    }
});
