[![GitHub Actions Status](https://github.com/salvo-rs/multra/actions/workflows/test.yml/badge.svg)](https://github.com/salvo-rs/multra/actions)
[![crates.io](https://img.shields.io/crates/v/multra.svg)](https://crates.io/crates/multra)
[![Documentation](https://docs.rs/multra/badge.svg)](https://docs.rs/multra)
[![MIT](https://img.shields.io/crates/l/multra.svg)](./LICENSE)

# multra

An async parser for `multipart/form-data` content-type in Rust. Forked from [multer](https://github.com/rwf2/multer).

It accepts a [`Stream`](https://docs.rs/futures/0.3/futures/stream/trait.Stream.html) of [`Bytes`](https://docs.rs/bytes/1/bytes/struct.Bytes.html) as
a source, so that It can be plugged into any async Rust environment e.g. any async server.

[Docs](https://docs.rs/multra)

## Install    

Add this to your `Cargo.toml`:

```toml
[dependencies]
multra = "3.1"
```

# Basic Example

```rust
use bytes::Bytes;
use futures::stream::Stream;
// Import multra types.
use multra::{Constraints, Multipart, SizeLimit};
use std::convert::Infallible;
use futures::stream::once;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Generate a byte stream and the boundary from somewhere e.g. server request body.
    let (stream, boundary) = get_byte_stream_from_somewhere().await;

    let constraints = Constraints::new().size_limit(
        SizeLimit::new()
            .whole_stream(15 * 1024 * 1024)
            .per_field(10 * 1024 * 1024)
            .for_field("my_text_field", 30 * 1024),
    );

    // Create a constrained `Multipart` instance for untrusted input.
    let mut multipart = Multipart::with_constraints(stream, boundary, constraints);

    // Iterate over the fields, use `next_field()` to get the next field.
    while let Some(mut field) = multipart.next_field().await? {
        // Get field name.
        let name = field.name();
        // Get the field's filename if provided in "Content-Disposition" header.
        let file_name = field.file_name();

        println!("Name: {:?}, File Name: {:?}", name, file_name);

        // Process the field data chunks e.g. store them in a file.
        while let Some(chunk) = field.chunk().await? {
            // Do something with field chunk.
            println!("Chunk: {:?}", chunk);
        }
    }

    Ok(())
}

// Generate a byte stream and the boundary from somewhere e.g. server request body.
async fn get_byte_stream_from_somewhere() -> (impl Stream<Item = Result<Bytes, Infallible>>, &'static str) {
    let data = "--X-BOUNDARY\r\nContent-Disposition: form-data; name=\"my_text_field\"\r\n\r\nabcd\r\n--X-BOUNDARY--\r\n";
    let stream = once(async move { Result::<Bytes, Infallible>::Ok(Bytes::from(data)) });
    
    (stream, "X-BOUNDARY")
}
``` 

`Multipart::new()` and `Multipart::with_reader()` keep backward-compatible unbounded
size limits. For untrusted uploads, prefer the constrained constructors.

## Prevent Denial of Service (DoS) Attacks

This crate provides APIs to prevent potential DoS attacks with fine grained control.
The default constructors leave stream and field size limits unbounded, so it is
recommended to add explicit constraints for untrusted multipart bodies.

An example:

```rust
use multra::{Constraints, Multipart, SizeLimit};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create some constraints to be applied to the fields to prevent DoS attack.
    let constraints = Constraints::new()
         // We only accept `my_text_field` and `my_file_field` fields,
         // For any unknown field, we will throw an error.
         .allowed_fields(vec!["my_text_field", "my_file_field"])
         .size_limit(
             SizeLimit::new()
                 // Set 15mb as size limit for the whole stream body.
                 .whole_stream(15 * 1024 * 1024)
                 // Set 10mb as size limit for all fields.
                 .per_field(10 * 1024 * 1024)
                 // Set 30kb as size limit for our text field only.
                 .for_field("my_text_field", 30 * 1024),
         );

    // Create a `Multipart` instance from a stream and the constraints.
    let mut multipart = Multipart::with_constraints(some_stream, "X-BOUNDARY", constraints);

    while let Some(field) = multipart.next_field().await.unwrap() {
        let content = field.text().await.unwrap();
        assert_eq!(content, "abcd");
    } 
   
    Ok(())
}
```

## Usage with [hyper.rs](https://hyper.rs/) server

An [example](https://github.com/salvo-rs/multra/blob/main/examples/hyper_server_example.rs) showing usage with [hyper.rs](https://hyper.rs/).

For more examples, please visit [examples](https://github.com/salvo-rs/multra/tree/main/examples).

## Contributing

Your PRs and suggestions are always welcome.
