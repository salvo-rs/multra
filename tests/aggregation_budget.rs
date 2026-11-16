use std::convert::Infallible;
use std::future::Future;
use std::pin::pin;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::task::{Context, Poll, Wake, Waker};

use bytes::Bytes;
use futures_util::stream;
use multra::Multipart;

struct WakeCounter(AtomicUsize);
impl Wake for WakeCounter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[tokio::test]
async fn aggregation_bounds_work_and_wakes_before_continuing() {
    const CHUNKS: usize = 200;
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = polls.clone();
    let source = stream::poll_fn(move |_| {
        let index = counter.fetch_add(1, Ordering::Relaxed);
        let data = match index {
            0 => Bytes::from_static(b"--B\r\nContent-Disposition: form-data; name=x\r\n\r\n"),
            1..=CHUNKS => Bytes::from_static(b"x"),
            _ => Bytes::from_static(b"\r\n--B--\r\n"),
        };
        Poll::Ready(Some(Ok::<_, Infallible>(data)))
    });
    let mut parser = Multipart::new(source, "B");
    let field = parser.next_field().await.unwrap().unwrap();
    let scheduled = Arc::new(WakeCounter(AtomicUsize::new(0)));
    let waker = Waker::from(scheduled.clone());
    let mut cx = Context::from_waker(&waker);
    let mut future = pin!(field.bytes());
    let mut pending_polls = 0;
    loop {
        let before = polls.load(Ordering::Relaxed);
        let wake_before = scheduled.0.load(Ordering::Relaxed);
        let result = future.as_mut().poll(&mut cx);
        assert!(polls.load(Ordering::Relaxed) - before <= 32);
        match result {
            Poll::Pending => {
                assert!(scheduled.0.load(Ordering::Relaxed) > wake_before);
                pending_polls += 1;
                assert!(pending_polls < CHUNKS);
            }
            Poll::Ready(result) => {
                assert_eq!(result.unwrap().as_ref(), vec![b'x'; CHUNKS]);
                assert!(pending_polls > 0);
                break;
            }
        }
    }
}
