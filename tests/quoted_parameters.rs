use std::convert::Infallible;

use bytes::Bytes;
use futures_util::stream;
use multra::{Constraints, Error, Multipart, SizeLimit};

#[tokio::test]
async fn quoted_parameters_decode_pairs_before_percent_encoding_at_every_split() {
    for (disposition, name, filename) in [
        (
            r#"form-data; name="a\\b"; filename="C:\\fakepath\\a.txt""#,
            r"a\b",
            r"C:\fakepath\a.txt",
        ),
        (
            r#"form-data; name="ad\min"; filename="a\;b.txt""#,
            "admin",
            "a;b.txt",
        ),
        (
            r#"form-data; name="a\"b"; filename="a\\\"b.txt""#,
            "a\"b",
            "a\\\"b.txt",
        ),
        (
            r#"form-data; name="f%20"; filename="\"%5C%22""#,
            "f%20",
            "\"\\\"",
        ),
        (
            r#"form-data; name="你好\\x"; filename="file\\""#,
            "你好\\x",
            "file\\",
        ),
        (
            r"form-data; name=raw\name; filename=C:\fakepath\file.txt",
            r"raw\name",
            r"C:\fakepath\file.txt",
        ),
    ] {
        let data = format!("--B\r\nContent-Disposition: {disposition}\r\n\r\nx\r\n--B--\r\n");
        for split in 0..=data.len() {
            let source = stream::iter([
                Ok::<_, Infallible>(Bytes::copy_from_slice(&data.as_bytes()[..split])),
                Ok(Bytes::copy_from_slice(&data.as_bytes()[split..])),
            ]);
            let mut multipart = Multipart::new(source, "B");
            let field = multipart.next_field().await.unwrap().unwrap();
            assert_eq!(field.name(), Some(name), "{disposition}, split={split}");
            assert_eq!(
                field.file_name(),
                Some(filename),
                "{disposition}, split={split}"
            );
            assert_eq!(field.text().await.unwrap(), "x");
            assert!(multipart.next_field().await.unwrap().is_none());
        }
    }
}

#[tokio::test]
async fn constraints_match_the_unquoted_field_name() {
    let data = Bytes::from_static(
        b"--B\r\nContent-Disposition: form-data; name=\"ad\\min\"\r\n\r\n1234\r\n--B--\r\n",
    );
    let constraints = Constraints::new()
        .allowed_fields(vec!["admin"])
        .size_limit(SizeLimit::new().for_field("admin", 1));
    let mut multipart =
        Multipart::with_constraints(stream::iter([Ok::<_, Infallible>(data)]), "B", constraints);
    let field = multipart.next_field().await.unwrap().unwrap();
    assert_eq!(field.name(), Some("admin"));
    assert!(matches!(
        field.bytes().await,
        Err(Error::FieldSizeExceeded { limit: 1, field_name: Some(name) }) if name == "admin"
    ));
}
