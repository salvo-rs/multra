use std::alloc::{GlobalAlloc, Layout, System};
use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use bytes::Bytes;
use futures_util::stream;
use multra::{Constraints, Error, Multipart, SizeLimit};

struct AllocationProbe;

static TRACK: AtomicBool = AtomicBool::new(false);
static LARGEST_REQUEST: AtomicUsize = AtomicUsize::new(0);

fn record(size: usize) {
    if TRACK.load(Ordering::Relaxed) {
        LARGEST_REQUEST.fetch_max(size, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for AllocationProbe {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        // SAFETY: Forward the caller's allocator contract unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        // SAFETY: Forward the caller's allocator contract unchanged.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Forward the caller's allocator contract unchanged.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record(new_size);
        // SAFETY: Forward the caller's allocator contract unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: AllocationProbe = AllocationProbe;

#[tokio::test]
async fn stage_limits_do_not_copy_oversized_source_chunks() {
    const HUGE: usize = 8 * 1024 * 1024;
    const OPENING: &[u8] = b"--B\r\nContent-Disposition: form-data; name=f\r\n\r\n";

    for scenario in ["preamble", "headers", "field", "whole-stream"] {
        for shared in [false, true] {
            for fragmented in [false, true] {
                // Construct the source before tracking parser allocations.
                let mut data = match scenario {
                    "headers" => b"--B\r\nX-Long: ".to_vec(),
                    "field" => OPENING.to_vec(),
                    _ => Vec::new(),
                };
                data.resize(HUGE, b'x');
                if scenario == "field" {
                    data.extend_from_slice(b"\r\n--B--\r\n");
                }
                let mut data = Bytes::from(data);
                let retained = shared.then(|| data.clone());
                let split = if fragmented {
                    if scenario == "field" {
                        OPENING.len() - 1
                    } else {
                        1
                    }
                } else {
                    0
                };
                let source = stream::iter([Ok::<_, Infallible>(data.split_to(split)), Ok(data)]);
                let limits = match scenario {
                    "preamble" => SizeLimit::new().preamble(64),
                    "headers" => SizeLimit::new().headers(64),
                    "field" => SizeLimit::new().per_field(64),
                    _ => SizeLimit::new().whole_stream(64),
                };
                let mut multipart =
                    Multipart::with_constraints(source, "B", Constraints::new().size_limit(limits));

                LARGEST_REQUEST.store(0, Ordering::Relaxed);
                TRACK.store(true, Ordering::Relaxed);
                let result = match multipart.next_field().await {
                    Ok(Some(field)) => field.bytes().await.map(|_| ()),
                    Ok(None) => Ok(()),
                    Err(error) => Err(error),
                };
                TRACK.store(false, Ordering::Relaxed);
                let largest = LARGEST_REQUEST.load(Ordering::Relaxed);

                match scenario {
                    "preamble" => assert!(matches!(
                        result,
                        Err(Error::PreambleSizeExceeded { limit: 64 })
                    )),
                    "headers" => assert!(matches!(
                        result,
                        Err(Error::HeadersSizeExceeded { limit: 64 })
                    )),
                    "field" => assert!(matches!(
                        result,
                        Err(Error::FieldSizeExceeded { limit: 64, .. })
                    )),
                    _ => assert!(matches!(
                        result,
                        Err(Error::StreamSizeExceeded { limit: 64 })
                    )),
                }
                assert!(
                    largest < 128 * 1024,
                    "{scenario}, shared={shared}, fragmented={fragmented}: parser allocated {largest} bytes"
                );
                drop(retained);
            }
        }
    }

    // A complete large field should still share the source allocation, even
    // when the source Bytes has another owner.
    let mut data = OPENING.to_vec();
    data.resize(data.len() + HUGE, b'x');
    data.extend_from_slice(b"\r\n--B--\r\n");
    let retained = Bytes::from(data);
    let expected_ptr = retained.as_ptr().wrapping_add(OPENING.len());
    let source = stream::iter([Ok::<_, Infallible>(retained.clone())]);
    let mut multipart = Multipart::new(source, "B");
    let body = multipart
        .next_field()
        .await
        .unwrap()
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(body.len(), HUGE);
    assert_eq!(body.as_ptr(), expected_ptr);
    assert!(multipart.next_field().await.unwrap().is_none());
    drop(multipart);
    assert!(body.iter().all(|byte| *byte == b'x'));
}
