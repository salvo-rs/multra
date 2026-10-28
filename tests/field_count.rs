use std::convert::Infallible;
use std::error::Error as _;
use std::fmt::Write as _;

use bytes::Bytes;
use futures_util::{StreamExt, stream};
use multra::{Constraints, Error, Multipart, SizeLimit};

fn multipart(data: &str, constraints: Constraints, chunk_size: usize) -> Multipart<'static> {
    let chunks: Vec<_> = data
        .as_bytes()
        .chunks(chunk_size)
        .map(|chunk| Ok::<_, Infallible>(Bytes::copy_from_slice(chunk)))
        .collect();
    Multipart::with_constraints(stream::iter(chunks), "B", constraints)
}

fn fields(names: &[&str]) -> String {
    let mut data = String::new();
    for name in names {
        write!(
            data,
            "--B\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n\r\n"
        )
        .unwrap();
    }
    data.push_str("--B--\r\n");
    data
}

#[tokio::test]
async fn zero_limit_accepts_empty_multipart_only() {
    for chunk_size in [1, 1024] {
        let mut empty = multipart("--B--\r\n", Constraints::new().max_fields(0), chunk_size);
        assert!(empty.next_field().await.unwrap().is_none());
        assert!(empty.next_field().await.unwrap().is_none());

        let mut one = multipart(
            &fields(&["a"]),
            Constraints::new().max_fields(0),
            chunk_size,
        );
        assert!(matches!(
            one.next_field().await,
            Err(Error::FieldCountExceeded { limit: 0 })
        ));
    }
}

#[tokio::test]
async fn exact_count_accepts_empty_fields_and_preserves_indices() {
    let data = fields(&["a", "a", "b"]);
    for chunk_size in [1, 3, data.len()] {
        let mut multipart = multipart(&data, Constraints::new().max_fields(3), chunk_size);
        for (index, name) in ["a", "a", "b"].into_iter().enumerate() {
            let (actual_index, field) = multipart.next_field_with_idx().await.unwrap().unwrap();
            assert_eq!(actual_index, index);
            assert_eq!(field.name(), Some(name));
            assert!(field.bytes().await.unwrap().is_empty());
        }
        assert!(multipart.next_field().await.unwrap().is_none());
    }
}

#[tokio::test]
async fn repeated_names_are_counted_and_limit_cannot_be_retried_away() {
    let data = fields(&["a", "a"]);
    for chunk_size in [1, data.len()] {
        let mut multipart = multipart(&data, Constraints::new().max_fields(1), chunk_size);
        multipart
            .next_field()
            .await
            .unwrap()
            .unwrap()
            .bytes()
            .await
            .unwrap();
        for _ in 0..2 {
            assert!(matches!(
                multipart.next_field().await,
                Err(Error::FieldCountExceeded { limit: 1 })
            ));
        }
    }
}

#[tokio::test]
async fn dropping_unread_fields_does_not_reset_the_count() {
    let data = "--B\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\nfirst\r\n--B\r\nContent-Disposition: form-data; name=\"b\"\r\n\r\nsecond\r\n--B--\r\n";
    for chunk_size in [1, 3, data.len()] {
        let mut multipart = multipart(data, Constraints::new().max_fields(1), chunk_size);
        drop(multipart.next_field().await.unwrap().unwrap());
        assert!(matches!(
            multipart.next_field().await,
            Err(Error::FieldCountExceeded { limit: 1 })
        ));
    }
}

#[tokio::test]
async fn excess_part_is_rejected_before_its_headers_arrive() {
    let data =
        Bytes::from_static(b"--B\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\n\r\n--B\r\n");
    let source = stream::iter([Ok::<_, Infallible>(data)]).chain(stream::pending());
    let mut multipart = Multipart::with_constraints(source, "B", Constraints::new().max_fields(1));
    drop(multipart.next_field().await.unwrap().unwrap());
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), multipart.next_field())
        .await
        .expect("field count must be checked before waiting for another header block");
    assert!(matches!(
        result,
        Err(Error::FieldCountExceeded { limit: 1 })
    ));
}

#[tokio::test]
async fn default_and_maximum_limits_preserve_compatible_behavior() {
    let names = ["a"; 128];
    let data = fields(&names);
    for constraints in [
        Constraints::new(),
        Constraints::new().max_fields(usize::MAX),
    ] {
        let mut multipart = multipart(&data, constraints, 3);
        let mut count = 0;
        while let Some(field) = multipart.next_field().await.unwrap() {
            assert_eq!(field.index(), count);
            drop(field);
            count += 1;
        }
        assert_eq!(count, names.len());
    }
}

#[tokio::test]
async fn builder_order_preserves_all_constraints() {
    let data = fields(&["a", "a"]);
    let cases = [
        Constraints::new()
            .max_fields(1)
            .size_limit(SizeLimit::new().per_field(0))
            .allowed_fields(vec!["a"]),
        Constraints::new()
            .allowed_fields(vec!["a"])
            .max_fields(1)
            .size_limit(SizeLimit::new().per_field(0)),
        Constraints::new()
            .size_limit(SizeLimit::new().per_field(0))
            .allowed_fields(vec!["a"])
            .max_fields(1),
    ];
    for constraints in cases {
        let mut multipart = multipart(&data, constraints, 1);
        drop(multipart.next_field().await.unwrap().unwrap());
        assert!(matches!(
            multipart.next_field().await,
            Err(Error::FieldCountExceeded { limit: 1 })
        ));
    }

    let mut unknown = multipart(
        &fields(&["b"]),
        Constraints::new().allowed_fields(vec!["a"]).max_fields(2),
        1,
    );
    assert!(matches!(
        unknown.next_field().await,
        Err(Error::UnknownField { .. })
    ));
    let mut too_large = multipart(
        "--B\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\nx\r\n--B--\r\n",
        Constraints::new()
            .size_limit(SizeLimit::new().per_field(0))
            .max_fields(2),
        1,
    );
    assert!(matches!(
        too_large.next_field().await.unwrap().unwrap().bytes().await,
        Err(Error::FieldSizeExceeded { .. })
    ));
}

#[test]
fn field_count_error_preserves_limit_and_has_no_source() {
    let error = Error::FieldCountExceeded { limit: 3 };
    assert_eq!(error, Error::FieldCountExceeded { limit: 3 });
    assert_ne!(error, Error::FieldCountExceeded { limit: 4 });
    assert_eq!(error.to_string(), "multipart field count exceeded limit: 3");
    assert!(error.source().is_none());
}
