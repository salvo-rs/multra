use std::convert::Infallible;

use bytes::Bytes;
use futures_util::stream;
use multra::{Constraints, Multipart};

#[tokio::test]
async fn repeated_headers_remain_visible_and_metadata_uses_the_last_value() {
    let body = b"--B\r\nContent-Disposition: form-data; name=first; filename=first.txt\r\nContent-Disposition: form-data; name=last; filename=last.txt\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Type: text/plain; charset=windows-1252\r\nX-Repeated: first\r\nX-Repeated: last\r\n\r\n\xe9\r\n--B--\r\n";
    for width in [1, 2, body.len()] {
        let chunks = body
            .chunks(width)
            .map(|chunk| Ok::<_, Infallible>(Bytes::copy_from_slice(chunk)));
        let mut multipart = Multipart::with_constraints(
            stream::iter(chunks),
            "B",
            Constraints::new().allowed_fields(vec!["last"]),
        );
        let field = multipart.next_field().await.unwrap().unwrap();
        assert_eq!(field.name(), Some("last"));
        assert_eq!(field.file_name(), Some("last.txt"));
        assert_eq!(
            field
                .content_type()
                .unwrap()
                .get_param(mime::CHARSET)
                .unwrap(),
            "windows-1252"
        );
        for (name, expected) in [
            (
                "content-disposition",
                [
                    "form-data; name=first; filename=first.txt",
                    "form-data; name=last; filename=last.txt",
                ],
            ),
            (
                "content-type",
                [
                    "text/plain; charset=utf-8",
                    "text/plain; charset=windows-1252",
                ],
            ),
            ("x-repeated", ["first", "last"]),
        ] {
            let actual: Vec<_> = field
                .headers()
                .get_all(name)
                .iter()
                .map(|value| value.to_str().unwrap())
                .collect();
            assert_eq!(actual, expected);
        }
        assert_eq!(field.text().await.unwrap(), "\u{e9}");
        assert!(multipart.next_field().await.unwrap().is_none());
    }
}

#[tokio::test]
async fn invalid_last_single_value_headers_do_not_fall_back_to_earlier_values() {
    let body = Bytes::from_static(
        b"--B\r\nContent-Disposition: form-data; name=first\r\nContent-Disposition: attachment\r\nContent-Type: text/plain\r\nContent-Type: invalid\r\n\r\nx\r\n--B--\r\n",
    );
    let mut multipart = Multipart::new(stream::iter([Ok::<_, Infallible>(body)]), "B");
    let field = multipart.next_field().await.unwrap().unwrap();
    assert!(field.name().is_none());
    assert!(field.content_type().is_none());
    assert_eq!(
        field
            .headers()
            .get_all("content-disposition")
            .iter()
            .count(),
        2
    );
    assert_eq!(field.headers().get_all("content-type").iter().count(), 2);
    assert_eq!(field.bytes().await.unwrap(), "x");
}
