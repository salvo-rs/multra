use std::convert::Infallible;
use std::future::Future;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::task::Poll;

use bytes::Bytes;
use futures_util::stream;
use multra::Multipart;

async fn poll_once<F: Future>(future: F) -> Poll<F::Output> {
    let mut future = std::pin::pin!(future);
    std::future::poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx))).await
}

#[tokio::test]
async fn test_buffered_fields_do_not_pull_more_upstream_chunks() {
    let chunk = Bytes::from(
        "--X-BOUNDARY\r\nContent-Disposition: form-data; name=\"field\"\r\n\r\nvalue\r\n"
            .repeat(256),
    );
    let chunk_len = chunk.len();
    let bytes_read = Arc::new(AtomicUsize::new(0));
    let read_count = bytes_read.clone();
    let stream = stream::poll_fn(move |_cx| {
        read_count.fetch_add(chunk.len(), Ordering::SeqCst);
        Poll::Ready(Some(Ok::<Bytes, Infallible>(chunk.clone())))
    });
    let mut multipart = Multipart::new(stream, "X-BOUNDARY");

    for index in 0..100 {
        let field = multipart.next_field().await.unwrap().unwrap();
        assert_eq!(field.index(), index);
        if index % 2 == 0 {
            assert_eq!(field.text().await.unwrap(), "value");
        }
        // Alternate reading and dropping fields to exercise both consumers.
        assert_eq!(bytes_read.load(Ordering::SeqCst), chunk_len);
    }
}

#[tokio::test]
async fn test_pending_stream_resumes_for_dropped_and_read_fields() {
    let data = b"--X-BOUNDARY\r\nContent-Disposition: form-data; name=\"first\"\r\n\r\nignored\r\n--X-BOUNDARY\r\nContent-Disposition: form-data; name=\"second\"\r\n\r\nvalue\r\n--X-BOUNDARY--";
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let pending_polls = Arc::new(AtomicUsize::new(0));
    let pending_count = pending_polls.clone();
    let stream = stream::poll_fn(move |cx| {
        let result = receiver.poll_recv(cx);
        if result.is_pending() {
            pending_count.fetch_add(1, Ordering::SeqCst);
        }
        result
    });
    let mut multipart = Multipart::new(stream, "X-BOUNDARY");

    // Register the real channel waker before the producer starts.
    assert!(poll_once(multipart.next_field()).await.is_pending());
    let producer = tokio::spawn(async move {
        for byte in data {
            sender
                .send(Ok::<Bytes, Infallible>(Bytes::copy_from_slice(&[*byte])))
                .unwrap();
            tokio::task::yield_now().await;
        }
    });

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let first = multipart.next_field().await.unwrap().unwrap();
        assert_eq!(first.name(), Some("first"));
        drop(first);
        let second = multipart.next_field().await.unwrap().unwrap();
        assert_eq!(second.name(), Some("second"));
        assert_eq!(second.text().await.unwrap(), "value");
        assert!(multipart.next_field().await.unwrap().is_none());
    })
    .await
    .unwrap();
    producer.await.unwrap();
    assert!(pending_polls.load(Ordering::SeqCst) > 0);
}

#[tokio::test]
async fn test_cancelled_reads_preserve_partial_headers_and_delimiters() {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let stream = stream::poll_fn(move |cx| receiver.poll_recv(cx));
    let mut multipart = Multipart::new(stream, "X-BOUNDARY");
    sender
        .send(Ok::<Bytes, Infallible>(Bytes::from_static(
            b"--X-BOUNDARY\r\nContent-Disposition: form-data; name=\"field\"\r\n",
        )))
        .unwrap();

    // Each temporary future is cancelled while the parser needs more input.
    assert!(poll_once(multipart.next_field()).await.is_pending());
    assert!(poll_once(multipart.next_field()).await.is_pending());
    sender
        .send(Ok(Bytes::from_static(b"\r\nvalue\r\n--X-BOUNDARY--")))
        .unwrap();
    let mut field = multipart.next_field().await.unwrap().unwrap();
    assert_eq!(field.chunk().await.unwrap().unwrap(), "value");
    assert!(poll_once(field.chunk()).await.is_pending());
    assert!(poll_once(field.chunk()).await.is_pending());

    // A closing delimiter without CRLF becomes valid only at actual EOF.
    drop(sender);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        assert!(field.chunk().await.unwrap().unwrap().is_empty());
        assert!(field.chunk().await.unwrap().is_none());
        drop(field);
        assert!(multipart.next_field().await.unwrap().is_none());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_truncated_stream_eof_is_reported_without_another_wake() {
    for data in [
        "",
        "--X-BOUNDARY",
        "--X-BOUNDARY\r",
        "--X-BOUNDARY\r\nContent-Disposition: form-data; name=\"field\"\r\n",
        "--X-BOUNDARY\r\nContent-Disposition: form-data; name=\"field\"\r\n\r\nvalue",
    ] {
        let stream = stream::once(async move { Ok::<Bytes, Infallible>(Bytes::from(data)) });
        let mut multipart = Multipart::new(stream, "X-BOUNDARY");
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            match multipart.next_field().await {
                Err(_) => {}
                Ok(Some(field)) => assert!(field.bytes().await.is_err()),
                Ok(None) => panic!("accepted truncated multipart data"),
            }
        })
        .await
        .unwrap();
    }
}
