use std::convert::Infallible;

use futures_util::stream;
use multra::{Constraints, Multipart, SizeLimit, bytes::Bytes};

const BODY: &[u8] = b"prefix--Xbogus\r\nprefix--X--junk\r\n--X\r\nContent-Disposition: form-data; name=\"f\"\r\n\r\nx\r\n--X--\r\n";

#[tokio::test]
async fn preamble_markers_inside_lines_are_ignored_at_every_split() {
    for split in 0..=BODY.len() {
        let source = stream::iter([
            Ok::<_, Infallible>(Bytes::from_static(&BODY[..split])),
            Ok(Bytes::from_static(&BODY[split..])),
        ]);
        let mut multipart = Multipart::new(source, "X");
        let field = multipart.next_field().await.unwrap().unwrap();
        assert_eq!(field.name(), Some("f"));
        assert_eq!(field.bytes().await.unwrap(), "x");
        assert!(multipart.next_field().await.unwrap().is_none());
    }
}

#[tokio::test]
async fn line_start_and_marker_can_be_split_into_individual_bytes() {
    let source = stream::iter(
        BODY.chunks(1)
            .map(|chunk| Ok::<_, Infallible>(Bytes::from_static(chunk))),
    );
    let mut multipart = Multipart::new(source, "X");
    assert_eq!(
        multipart
            .next_field()
            .await
            .unwrap()
            .unwrap()
            .bytes()
            .await
            .unwrap(),
        "x"
    );
    assert!(multipart.next_field().await.unwrap().is_none());
}

#[tokio::test]
async fn ignored_inline_markers_still_count_against_preamble_limit() {
    let source = stream::iter([Ok::<_, Infallible>(Bytes::from_static(BODY))]);
    let constraints = Constraints::new().size_limit(SizeLimit::new().preamble(10));
    let mut multipart = Multipart::with_constraints(source, "X", constraints);
    assert!(matches!(
        multipart.next_field().await,
        Err(multra::Error::PreambleSizeExceeded { limit: 10 })
    ));
}
