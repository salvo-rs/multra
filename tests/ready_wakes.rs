use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::task::{Context, Poll, Wake, Waker};

use bytes::Bytes;
use futures_util::{Stream, stream};
use multra::Multipart;

struct WakeCounter(AtomicUsize);
impl Wake for WakeCounter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn ready_field_and_chunk_reads_do_not_schedule_another_poll() {
    let source = stream::iter([
        Ok::<_, Infallible>(Bytes::from_static(
            b"--B\r\nContent-Disposition: form-data; name=x\r\n\r\n",
        )),
        Ok(Bytes::from_static(b"data\r\n--B--\r\n")),
    ]);
    let mut parser = Multipart::new(source, "B");
    let scheduled = Arc::new(WakeCounter(AtomicUsize::new(0)));
    let waker = Waker::from(scheduled.clone());
    let mut cx = Context::from_waker(&waker);
    let Poll::Ready(Ok(Some(mut field))) = parser.poll_next_field(&mut cx) else {
        panic!("field should be ready");
    };
    assert_eq!(scheduled.0.load(Ordering::Relaxed), 0);
    assert!(
        matches!(Pin::new(&mut field).poll_next(&mut cx), Poll::Ready(Some(Ok(bytes))) if bytes == "data")
    );
    assert_eq!(scheduled.0.load(Ordering::Relaxed), 0);
}

#[test]
fn incomplete_ready_input_still_schedules_a_repoll() {
    let source = stream::iter([
        Ok::<_, Infallible>(Bytes::from_static(
            b"--B\r\nContent-Disposition: form-data; name=",
        )),
        Ok(Bytes::from_static(b"x\r\n\r\ndata\r\n--B--\r\n")),
    ]);
    let mut parser = Multipart::new(source, "B");
    let scheduled = Arc::new(WakeCounter(AtomicUsize::new(0)));
    let waker = Waker::from(scheduled.clone());
    let mut cx = Context::from_waker(&waker);
    assert!(parser.poll_next_field(&mut cx).is_pending());
    assert_eq!(scheduled.0.load(Ordering::Relaxed), 1);
    assert!(matches!(
        parser.poll_next_field(&mut cx),
        Poll::Ready(Ok(Some(_)))
    ));
    assert_eq!(scheduled.0.load(Ordering::Relaxed), 1);
}

#[test]
fn upstream_pending_does_not_create_a_busy_loop() {
    let mut parser = Multipart::new(stream::pending::<Result<Bytes, Infallible>>(), "B");
    let scheduled = Arc::new(WakeCounter(AtomicUsize::new(0)));
    let waker = Waker::from(scheduled.clone());
    let mut cx = Context::from_waker(&waker);
    assert!(parser.poll_next_field(&mut cx).is_pending());
    assert_eq!(scheduled.0.load(Ordering::Relaxed), 0);
}
