use std::convert::Infallible;
use std::future::Future;
use std::pin::pin;
use std::task::Context;
use std::task::Waker;
use std::time::Duration;

use bytes::Bytes;
use futures_util::stream;
use multra::{Constraints, Multipart, SizeLimit};

const BODY: &[u8] = b"--X-BOUNDARY\r\nContent-Disposition: form-data; name=\"text\"\r\n\r\nhello\r\n--X-BOUNDARY\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\n\r\na\0b\r\n--X-BOUNDARY--\r\n";

async fn collect(chunks: Vec<Bytes>) -> Vec<(String, Option<String>, Bytes)> {
    let source = stream::iter(chunks.into_iter().map(Ok::<_, Infallible>));
    let constraints =
        Constraints::new().size_limit(SizeLimit::new().whole_stream(1024).per_field(16));
    let mut multipart = Multipart::with_constraints(source, "X-BOUNDARY", constraints);
    let mut fields = Vec::new();
    while let Some(field) = multipart.next_field().await.unwrap() {
        let name = field.name().unwrap().to_owned();
        let filename = field.file_name().map(str::to_owned);
        fields.push((name, filename, field.bytes().await.unwrap()));
    }
    fields
}

#[tokio::test]
async fn field_values_are_independent_of_chunk_boundaries() {
    let expected = vec![
        ("text".to_owned(), None, Bytes::from_static(b"hello")),
        (
            "file".to_owned(),
            Some("a.txt".to_owned()),
            Bytes::from_static(b"a\0b"),
        ),
    ];
    for width in 1..=BODY.len() {
        let chunks = BODY.chunks(width).map(Bytes::copy_from_slice).collect();
        assert_eq!(collect(chunks).await, expected, "chunk width {width}");
    }
    for split in 0..=BODY.len() {
        let chunks = vec![
            Bytes::copy_from_slice(&BODY[..split]),
            Bytes::copy_from_slice(&BODY[split..]),
        ];
        assert_eq!(collect(chunks).await, expected, "split at {split}");
    }
}

#[tokio::test]
async fn pending_reads_resume_after_cancelling_field_futures() {
    tokio::time::timeout(Duration::from_secs(2), async {
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<Result<Bytes, Infallible>>(2);
        let source = stream::poll_fn(move |cx| receiver.poll_recv(cx));
        let mut multipart = Multipart::new(source, "X-BOUNDARY");
        sender
            .send(Ok(Bytes::from_static(
                b"--X-BOUNDARY\r\nContent-Disposition: form-data; name=\"up",
            )))
            .await
            .unwrap();
        let mut cx = Context::from_waker(Waker::noop());
        {
            let mut next = pin!(multipart.next_field());
            assert!(next.as_mut().poll(&mut cx).is_pending());
            // The next poll reaches the channel's actual Pending state.
            assert!(next.as_mut().poll(&mut cx).is_pending());
        }
        let continuation_sender = sender.clone();
        let continuation = tokio::spawn(async move {
            tokio::task::yield_now().await;
            continuation_sender
                .send(Ok(Bytes::from_static(b"load\"\r\n\r\nabc\r\n--X-BO")))
                .await
                .unwrap();
        });
        let mut field = multipart.next_field().await.unwrap().unwrap();
        continuation.await.unwrap();
        assert_eq!(field.name(), Some("upload"));
        assert_eq!(field.chunk().await.unwrap().unwrap(), "abc");
        {
            let mut chunk = pin!(field.chunk());
            assert!(chunk.as_mut().poll(&mut cx).is_pending());
        }
        let completion = tokio::spawn(async move {
            tokio::task::yield_now().await;
            sender
                .send(Ok(Bytes::from_static(b"UNDARY--\r\n")))
                .await
                .unwrap();
        });
        assert!(field.chunk().await.unwrap().unwrap().is_empty());
        completion.await.unwrap();
        assert!(field.chunk().await.unwrap().is_none());
        drop(field);
        assert!(multipart.next_field().await.unwrap().is_none());
    })
    .await
    .expect("parser failed to resume after cancellation");
}
