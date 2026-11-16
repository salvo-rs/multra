use std::convert::Infallible;

use bytes::Bytes;
use futures_util::stream;
use multra::{Error, Multipart};

#[tokio::test]
async fn test_multiple_field_headers_at_every_input_split() {
    let data = b"--X-BOUNDARY\r\nContent-Disposition: form-data; name=\"first\"\r\nX-Long: several bytes\r\n\r\none\r\n--X-BOUNDARY\r\nContent-Disposition: form-data; name=\"second\"\r\n\r\ntwo\r\n--X-BOUNDARY--\r\n";
    for split in 0..=data.len() {
        let stream = stream::iter([
            Ok::<Bytes, Infallible>(Bytes::copy_from_slice(&data[..split])),
            Ok(Bytes::copy_from_slice(&data[split..])),
        ]);
        let mut multipart = Multipart::new(stream, "X-BOUNDARY");
        for (name, value) in [("first", "one"), ("second", "two")] {
            let field = multipart.next_field().await.unwrap().unwrap();
            assert_eq!(field.name(), Some(name));
            assert_eq!(field.text().await.unwrap(), value);
        }
        assert!(multipart.next_field().await.unwrap().is_none());
    }
}

#[tokio::test]
async fn rejects_early_header_terminators_at_every_input_split() {
    for data in [
        &b"--B\r\nContent-Disposition: form-data; name=f\n\nLOST\r\n\r\nKEPT\r\n--B--\r\n"[..],
        &b"--B\r\nContent-Disposition: form-data; name=f\r\n\nLOST\r\n\r\nKEPT\r\n--B--\r\n"[..],
    ] {
        for split in 0..=data.len() {
            let source = stream::iter([
                Ok::<_, Infallible>(Bytes::copy_from_slice(&data[..split])),
                Ok(Bytes::copy_from_slice(&data[split..])),
            ]);
            let mut multipart = Multipart::new(source, "B");
            assert!(
                matches!(
                    multipart.next_field().await,
                    Err(Error::ReadHeaderFailed(httparse::Error::NewLine))
                ),
                "accepted an early header terminator at split {split}"
            );
        }
    }
}

#[tokio::test]
async fn preserves_blank_lines_in_field_data_at_every_input_split() {
    let data =
        b"--B\r\nContent-Disposition: form-data; name=f\r\n\r\nLOST\r\n\r\nKEPT\r\n--B--\r\n";
    for split in 0..=data.len() {
        let source = stream::iter([
            Ok::<_, Infallible>(Bytes::copy_from_slice(&data[..split])),
            Ok(Bytes::copy_from_slice(&data[split..])),
        ]);
        let mut multipart = Multipart::new(source, "B");
        let field = multipart.next_field().await.unwrap().unwrap();
        assert_eq!(field.name(), Some("f"));
        assert_eq!(field.bytes().await.unwrap(), "LOST\r\n\r\nKEPT");
        assert!(multipart.next_field().await.unwrap().is_none());
    }
}
