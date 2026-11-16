use std::convert::Infallible;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Waker};

use bytes::Bytes;
use futures_util::stream;
use multra::{Constraints, Error, Multipart, SizeLimit};

const BODY: &[u8] = b"--B\r\nContent-Disposition: form-data; name=x\r\n\r\nv\r\n--B--\r\nepilogue";

#[tokio::test]
async fn whole_stream_limit_counts_epilogue_at_every_split() {
    for split in 0..=BODY.len() {
        for read in [false, true] {
            let source = stream::iter([
                Ok::<_, Infallible>(Bytes::copy_from_slice(&BODY[..split])),
                Ok(Bytes::copy_from_slice(&BODY[split..])),
            ]);
            let limit = BODY.len() as u64 - 1;
            let mut parser = Multipart::with_constraints(
                source,
                "B",
                Constraints::new().size_limit(SizeLimit::new().whole_stream(limit)),
            );
            let result = async {
                while let Some(field) = parser.next_field().await? {
                    if read {
                        assert_eq!(field.bytes().await?.as_ref(), b"v");
                    }
                }
                Ok::<_, Error>(())
            }
            .await;
            assert!(
                matches!(result, Err(Error::StreamSizeExceeded { limit: actual }) if actual == limit),
                "split={split}, read={read}: {result:?}"
            );
        }
    }
}

#[tokio::test]
async fn exact_limit_accepts_epilogue_at_every_chunk_width() {
    for width in 1..=BODY.len() {
        let source = stream::iter(
            BODY.chunks(width)
                .map(Bytes::copy_from_slice)
                .map(Ok::<_, Infallible>),
        );
        let mut parser = Multipart::with_constraints(
            source,
            "B",
            Constraints::new().size_limit(SizeLimit::new().whole_stream(BODY.len() as u64)),
        );
        assert_eq!(
            parser
                .next_field()
                .await
                .unwrap()
                .unwrap()
                .text()
                .await
                .unwrap(),
            "v"
        );
        assert!(parser.next_field().await.unwrap().is_none());
    }
}

#[tokio::test]
async fn finite_limit_waits_for_eof_after_cancelling_epilogue_read() {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let source = stream::poll_fn(move |cx| receiver.poll_recv(cx));
    let mut parser = Multipart::with_constraints(
        source,
        "B",
        Constraints::new().size_limit(SizeLimit::new().whole_stream(32)),
    );
    sender
        .send(Ok::<_, Infallible>(Bytes::from_static(b"--B--\r\n")))
        .unwrap();
    {
        let mut next = pin!(parser.next_field());
        let mut cx = Context::from_waker(Waker::noop());
        assert!(next.as_mut().poll(&mut cx).is_pending());
    }
    sender.send(Ok(Bytes::from_static(b"tail"))).unwrap();
    drop(sender);
    assert!(parser.next_field().await.unwrap().is_none());
}

#[tokio::test]
async fn finite_limit_propagates_epilogue_source_errors() {
    let source = stream::iter([
        Ok(Bytes::from_static(b"--B--\r\n")),
        Err(std::io::Error::other("epilogue source failure")),
    ]);
    let mut parser = Multipart::with_constraints(
        source,
        "B",
        Constraints::new().size_limit(SizeLimit::new().whole_stream(32)),
    );
    assert!(matches!(
        parser.next_field().await,
        Err(Error::StreamReadFailed(_))
    ));
    let source = stream::iter([
        Ok(Bytes::from_static(b"--B--\r\n")),
        Err(std::io::Error::other("unread epilogue")),
    ]);
    assert!(
        Multipart::new(source, "B")
            .next_field()
            .await
            .unwrap()
            .is_none()
    );
}
