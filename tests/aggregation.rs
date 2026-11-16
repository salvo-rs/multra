use std::convert::Infallible;

use bytes::Bytes;
use futures_util::stream;
use multra::{Constraints, Error, Multipart, SizeLimit};

#[tokio::test]
async fn collects_empty_and_already_consumed_fields() {
    let input = Bytes::from_static(
        b"--X\r\nContent-Disposition: form-data; name=\"empty\"\r\n\r\n\r\n--X\r\nContent-Disposition: form-data; name=\"data\"\r\n\r\nvalue\r\n--X--\r\n",
    );
    let mut multipart = Multipart::new(stream::iter([Ok::<_, Infallible>(input)]), "X");
    assert!(
        multipart
            .next_field()
            .await
            .unwrap()
            .unwrap()
            .bytes()
            .await
            .unwrap()
            .is_empty()
    );
    let mut field = multipart.next_field().await.unwrap().unwrap();
    assert_eq!(field.chunk().await.unwrap().unwrap(), "value");
    assert!(field.bytes().await.unwrap().is_empty());
    assert!(multipart.next_field().await.unwrap().is_none());
}

#[tokio::test]
async fn collects_payload_at_every_input_split() {
    let input = b"--X\r\nContent-Disposition: form-data; name=\"data\"\r\n\r\nvalue\r\n--X--\r\n";
    for split in 0..=input.len() {
        let source = stream::iter([
            Ok::<_, Infallible>(Bytes::copy_from_slice(&input[..split])),
            Ok(Bytes::copy_from_slice(&input[split..])),
        ]);
        let mut multipart = Multipart::new(source, "X");
        let field = multipart.next_field().await.unwrap().unwrap();
        assert_eq!(field.bytes().await.unwrap(), "value");
        assert!(multipart.next_field().await.unwrap().is_none());
    }
}

#[tokio::test]
async fn collects_many_payload_chunks() {
    let mut chunks = vec![Ok::<_, Infallible>(Bytes::from_static(
        b"--X\r\nContent-Disposition: form-data; name=\"data\"\r\n\r\n",
    ))];
    for _ in 0..128 {
        chunks.push(Ok(Bytes::from_static(b"abc")));
        chunks.push(Ok(Bytes::new()));
    }
    chunks.push(Ok(Bytes::from_static(b"\r\n--X--\r\n")));
    let mut multipart = Multipart::new(stream::iter(chunks), "X");
    let field = multipart.next_field().await.unwrap().unwrap();
    assert_eq!(field.bytes().await.unwrap(), "abc".repeat(128));
}

#[tokio::test]
async fn propagates_source_and_size_errors_after_partial_data() {
    let opening =
        Bytes::from_static(b"--X\r\nContent-Disposition: form-data; name=\"data\"\r\n\r\nfirst");
    let source = stream::iter([
        Ok(opening.clone()),
        Err(std::io::Error::other("source failed")),
    ]);
    let mut multipart = Multipart::new(source, "X");
    let field = multipart.next_field().await.unwrap().unwrap();
    assert!(matches!(
        field.bytes().await,
        Err(Error::StreamReadFailed(_))
    ));

    let source = stream::iter([
        Ok::<_, Infallible>(opening),
        Ok(Bytes::from_static(b"second\r\n--X--\r\n")),
    ]);
    let constraints = Constraints::new().size_limit(SizeLimit::new().per_field(5));
    let mut multipart = Multipart::with_constraints(source, "X", constraints);
    let field = multipart.next_field().await.unwrap().unwrap();
    assert!(matches!(
        field.bytes().await,
        Err(Error::FieldSizeExceeded { limit: 5, .. })
    ));
}

#[tokio::test]
async fn collected_chunk_survives_reading_later_fields() {
    let first = Bytes::from_static(
        b"--X\r\nContent-Disposition: form-data; name=\"first\"\r\n\r\nkeep this\r\n--X\r\nContent-Disposition: form-data; name=\"second\"\r\n\r\n",
    );
    let mut second = vec![b'x'; 128 * 1024];
    second.extend_from_slice(b"\r\n--X--\r\n");
    let source = stream::iter([Ok::<_, Infallible>(first), Ok(Bytes::from(second))]);
    let mut multipart = Multipart::new(source, "X");
    let retained = multipart
        .next_field()
        .await
        .unwrap()
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let next = multipart
        .next_field()
        .await
        .unwrap()
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(next.len(), 128 * 1024);
    assert!(multipart.next_field().await.unwrap().is_none());
    drop(multipart);
    assert_eq!(retained, "keep this");
}

#[tokio::test]
async fn large_chunks_after_partial_headers_preserve_all_fields() {
    let mut opening = b"--X\r\nContent-Disposition: form-data; name=\"first\"\r\nX-Long: ".to_vec();
    // A valid header larger than the coalescing prefix needs several rounds
    // before the remaining source can be exposed as field data.
    opening.resize(opening.len() + 48 * 1024, b'h');
    opening.extend_from_slice(b"\r\n\r\n");
    let mut data = opening.clone();
    let payload = vec![b'x'; 128 * 1024];
    data.extend_from_slice(&payload);
    data.extend_from_slice(
        b"\r\n--X\r\nContent-Disposition: form-data; name=\"second\"\r\n\r\nlast\r\n--X--\r\n",
    );
    let data = Bytes::from(data);
    for split in [1, 4, opening.len() - 1, opening.len() + 1] {
        for skip_first in [false, true] {
            let source = stream::iter([
                Ok::<_, Infallible>(data.slice(..split)),
                Ok(data.slice(split..)),
            ]);
            let limits = SizeLimit::new()
                .whole_stream(data.len() as u64)
                .per_field(payload.len() as u64);
            let mut multipart =
                Multipart::with_constraints(source, "X", Constraints::new().size_limit(limits));
            let first = multipart.next_field().await.unwrap().unwrap();
            assert_eq!(first.name(), Some("first"));
            if skip_first {
                drop(first);
            } else {
                assert_eq!(first.bytes().await.unwrap().as_ref(), payload);
            }
            let second = multipart.next_field().await.unwrap().unwrap();
            assert_eq!(second.name(), Some("second"));
            assert_eq!(second.index(), 1);
            assert_eq!(second.bytes().await.unwrap(), "last");
            assert!(multipart.next_field().await.unwrap().is_none());
        }
    }
}
