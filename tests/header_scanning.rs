use std::convert::Infallible;

use bytes::Bytes;
use futures_util::stream;
use multra::Multipart;

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
