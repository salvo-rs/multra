use std::convert::Infallible;

use bytes::Bytes;
use futures_util::stream;
use multra::Multipart;

#[tokio::test]
async fn test_extended_filename_respects_the_declared_charset() {
    for (extended, expected) in [
        ("UTF-8''%EF%BB%BFfile.txt", "\u{feff}file.txt"),
        ("UTF-8''%FF%FEa%00", "fallback.txt"),
        ("ISO-8859-1''%EF%BB%BFfile.txt", "ï»¿file.txt"),
        ("ISO-8859-1''%80.txt", "\u{80}.txt"),
        ("iso-8859-1'en'%A3.txt", "£.txt"),
        ("UTF-8''%E4%BD%A0%E5%A5%BD.txt", "你好.txt"),
        ("windows-1252''%80.txt", "€.txt"),
        ("unknown''invalid", "fallback.txt"),
        ("UTF-8''%E4%BD", "fallback.txt"),
    ] {
        let data = format!(
            "--B\r\nContent-Disposition: form-data; name=upload; filename=fallback.txt; filename*={extended}\r\n\r\nx\r\n--B--\r\n"
        );
        let source = stream::once(async move { Ok::<_, Infallible>(Bytes::from(data)) });
        let mut multipart = Multipart::new(source, "B");
        let field = multipart.next_field().await.unwrap().unwrap();
        assert_eq!(field.file_name(), Some(expected), "filename*: {extended}");
        assert_eq!(field.text().await.unwrap(), "x");
        assert!(multipart.next_field().await.unwrap().is_none());
    }
}
