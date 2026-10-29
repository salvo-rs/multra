use std::convert::Infallible;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::task::{Context, Poll, Wake, Waker};

use futures_util::stream;
use multra::{Multipart, bytes::Bytes};

struct WakeCounter(AtomicUsize);

impl Wake for WakeCounter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn always_ready_empty_chunks_yield_and_schedule_another_poll() {
    let polls = Arc::new(AtomicUsize::new(0));
    let count = polls.clone();
    let source = stream::poll_fn(move |_| {
        count.fetch_add(1, Ordering::Relaxed);
        Poll::Ready(Some(Ok::<_, Infallible>(Bytes::new())))
    });
    let mut multipart = Multipart::new(source, "X");
    let scheduled = Arc::new(WakeCounter(AtomicUsize::new(0)));
    let waker = Waker::from(scheduled.clone());
    let mut cx = Context::from_waker(&waker);

    assert!(multipart.poll_next_field(&mut cx).is_pending());
    assert!(polls.load(Ordering::Relaxed) <= 64);
    assert!(scheduled.0.load(Ordering::Relaxed) > 0);
}

#[tokio::test]
async fn data_after_empty_chunks_remains_readable() {
    let empty = std::iter::repeat_n(Ok::<_, Infallible>(Bytes::new()), 200);
    let body = Bytes::from_static(
        b"--X\r\nContent-Disposition: form-data; name=\"f\"\r\n\r\nvalue\r\n--X--\r\n",
    );
    let source = stream::iter(empty.chain(std::iter::once(Ok(body))));
    let mut multipart = Multipart::new(source, "X");

    let field = multipart.next_field().await.unwrap().unwrap();
    assert_eq!(field.bytes().await.unwrap(), "value");
    assert!(multipart.next_field().await.unwrap().is_none());
}
