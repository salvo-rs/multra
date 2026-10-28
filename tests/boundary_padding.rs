use std::convert::Infallible;

use bytes::Bytes;
use futures_util::{StreamExt, stream};
use multra::{Constraints, Multipart, SizeLimit};

async fn padded_fields(
    data: &str,
    limits: SizeLimit,
    chunk_size: usize,
) -> multra::Result<Vec<String>> {
    let chunks: Vec<_> = data
        .as_bytes()
        .chunks(chunk_size)
        .map(|chunk| Ok::<_, Infallible>(Bytes::copy_from_slice(chunk)))
        .collect();
    let mut multipart = Multipart::with_constraints(
        stream::iter(chunks),
        "B",
        Constraints::new().size_limit(limits),
    );
    let mut fields = Vec::new();
    while let Some(field) = multipart.next_field().await? {
        fields.push(field.text().await?);
    }
    Ok(fields)
}

#[tokio::test]
async fn test_boundary_padding_limit_preserves_field_data_across_chunks() {
    let data = "--B \t \r\nContent-Disposition: form-data; name=\"a\"\r\n\r\n first \t\r\n--B\t \t\r\nContent-Disposition: form-data; name=\"b\"\r\n\r\n\tsecond \r\n--B-- \t \r\nepilogue";
    for chunk_size in [1, 2, 3, data.len()] {
        assert_eq!(
            padded_fields(data, SizeLimit::new().boundary_padding(3), chunk_size).await,
            Ok(vec![" first \t".to_owned(), "\tsecond ".to_owned()])
        );
    }
}

#[tokio::test]
async fn test_boundary_padding_limit_rejects_all_delimiters() {
    let headers = "Content-Disposition: form-data; name=\"a\"\r\n\r\n";
    let cases = [
        format!("--B \t  \r\n{headers}\r\n--B--\r\n"),
        format!("--B\r\n{headers}\r\n--B \t  \r\n{headers}\r\n--B--\r\n"),
        format!("--B\r\n{headers}\r\n--B-- \t  \r\n"),
        "--B-- \t  \r\n".to_owned(),
    ];
    for data in cases {
        for chunk_size in [1, 2, data.len()] {
            assert_eq!(
                padded_fields(
                    &data,
                    SizeLimit::new().per_field(0).boundary_padding(3),
                    chunk_size,
                )
                .await,
                Err(multra::Error::BoundaryPaddingSizeExceeded { limit: 3 }),
                "chunk size {chunk_size}, data {data:?}"
            );
        }
    }
}

#[tokio::test]
async fn test_default_boundary_padding_limit_bounds_unfinished_delimiters() {
    // No newline or EOF is needed to reject padding. Field limits do not
    // account for these bytes, so the independent limit must handle them.
    for prefix in [
        "--B".to_owned(),
        "--B--".to_owned(),
        "--B\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\n\r\n--B".to_owned(),
        "--B\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\n\r\n--B--".to_owned(),
    ] {
        let data = format!("{prefix}{}", " ".repeat(8193));
        let chunks: Vec<_> = data
            .as_bytes()
            .chunks(1)
            .map(|chunk| Ok::<_, Infallible>(Bytes::copy_from_slice(chunk)))
            .collect();
        let source = stream::iter(chunks).chain(stream::pending());
        let mut multipart = Multipart::with_constraints(
            source,
            "B",
            Constraints::new().size_limit(SizeLimit::new().per_field(0)),
        );
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while let Some(field) = multipart.next_field().await? {
                field.bytes().await?;
            }
            Ok::<_, multra::Error>(())
        })
        .await
        .expect("unfinished padding must fail before the source wakes again");
        assert_eq!(
            result,
            Err(multra::Error::BoundaryPaddingSizeExceeded { limit: 8192 })
        );
    }
}

#[tokio::test]
async fn test_boundary_padding_zero_and_eof_threshold() {
    for data in [
        "--B\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\n\r\n--B--",
        "--B--",
    ] {
        assert!(
            padded_fields(data, SizeLimit::new().boundary_padding(0), 1)
                .await
                .is_ok()
        );
    }
    assert_eq!(
        padded_fields("--B-- \t ", SizeLimit::new().boundary_padding(3), 1).await,
        Ok(vec![])
    );
    assert_eq!(
        padded_fields("--B-- ", SizeLimit::new().boundary_padding(0), 1).await,
        Err(multra::Error::BoundaryPaddingSizeExceeded { limit: 0 })
    );
    assert_eq!(
        padded_fields("--B-- \t  ", SizeLimit::new().boundary_padding(3), 1).await,
        Err(multra::Error::BoundaryPaddingSizeExceeded { limit: 3 })
    );
}

#[tokio::test]
async fn test_boundary_padding_limit_can_be_raised() {
    let padding = " ".repeat(8193);
    let headers = "Content-Disposition: form-data; name=\"a\"\r\n\r\n";
    let data = format!("--B{padding}\r\n{headers}\r\n--B{padding}\r\n{headers}\r\n--B--{padding}");
    assert_eq!(
        padded_fields(
            &data,
            SizeLimit::new().boundary_padding(8193).per_field(0),
            64,
        )
        .await,
        Ok(vec![String::new(), String::new()])
    );
}

#[tokio::test]
async fn test_boundary_padding_limit_applies_when_skipping_field_data() {
    for suffix in ["--B    \r\n", "--B--    \r\n"] {
        let data = format!("--B\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\n\r\n{suffix}");
        let chunks: Vec<_> = data
            .as_bytes()
            .chunks(1)
            .map(|chunk| Ok::<_, Infallible>(Bytes::copy_from_slice(chunk)))
            .collect();
        let mut multipart = Multipart::with_constraints(
            stream::iter(chunks),
            "B",
            Constraints::new().size_limit(SizeLimit::new().boundary_padding(3).per_field(0)),
        );
        drop(multipart.next_field().await.unwrap().unwrap());
        assert!(matches!(
            multipart.next_field().await,
            Err(multra::Error::BoundaryPaddingSizeExceeded { limit: 3 })
        ));
    }
}

#[tokio::test]
async fn test_whole_stream_limit_still_counts_discarded_boundary_padding() {
    assert_eq!(
        padded_fields(
            "--B         \r\n",
            SizeLimit::new().boundary_padding(100).whole_stream(8),
            1,
        )
        .await,
        Err(multra::Error::StreamSizeExceeded { limit: 8 })
    );
}

#[tokio::test]
async fn test_empty_closing_boundary_rejects_non_padding_suffix() {
    for data in ["--B--junk", "--B-- \tjunk", "--B--\r"] {
        for chunk_size in [1, data.len()] {
            assert_eq!(
                padded_fields(data, SizeLimit::new(), chunk_size).await,
                Err(multra::Error::IncompleteStream)
            );
        }
    }
}
