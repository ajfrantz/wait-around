#[cfg(feature = "no_std")]
extern crate alloc;
#[cfg(not(feature = "no_std"))]
extern crate std as alloc;

use alloc::{rc::Rc, vec::Vec};
use core::{cell::RefCell, pin::Pin};
use futures::{
    io::Error,
    prelude::*,
    task::{Context, Poll, Waker},
};

pub struct RingBuffer {
    data: Vec<u8>,
    read_idx: usize,
    write_idx: usize,
    waker: Option<Waker>,
}

impl RingBuffer {
    pub fn with_capacity(n: usize) -> (Writer, Reader) {
        let rb = Rc::new(RefCell::new(RingBuffer {
            data: vec![0; n],
            read_idx: 0,
            write_idx: 0,
            waker: None,
        }));
        (Writer { rb: rb.clone() }, Reader { rb })
    }

    fn wrap(&self, mut idx: usize) -> usize {
        let capacity = self.data.len();
        if idx >= capacity {
            idx -= capacity;
        }
        idx
    }

    fn read(&mut self, amount: usize) {
        self.read_idx += amount;

        let capacity = self.data.len();
        if self.read_idx >= 2 * capacity {
            self.read_idx -= 2 * capacity;
        }
    }

    fn wrote(&mut self, amount: usize) {
        self.write_idx += amount;

        let capacity = self.data.len();
        if self.write_idx >= 2 * capacity {
            self.write_idx -= 2 * capacity;
        }
    }

    fn readable(&self) -> usize {
        if self.read_idx == self.write_idx {
            return 0;
        }

        let read_idx = self.wrap(self.read_idx);
        let write_idx = self.wrap(self.write_idx);
        if read_idx < write_idx {
            // No wrapping, read valid data.
            //   [x r x w]
            //      ^--^
            write_idx - read_idx
        } else {
            // Write index has wrapped, read to end.
            //   [w x x x r x x]
            //    ^       ^----^
            self.data.len() - read_idx
        }
    }

    fn writeable(&self) -> usize {
        let capacity = self.data.len();
        let mut write_idx = self.write_idx;
        if write_idx < self.read_idx {
            write_idx += 2 * capacity;
        }

        let remaining_space = capacity - (write_idx - self.read_idx);
        let space_before_end = capacity - self.wrap(self.write_idx);
        remaining_space.min(space_before_end)
    }

    fn park(&mut self, waker: &Waker) {
        self.waker = Some(waker.clone());
    }

    fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

pub struct Reader {
    rb: Rc<RefCell<RingBuffer>>,
}

impl AsyncRead for Reader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut [u8],
    ) -> Poll<Result<usize, Error>> {
        let mut rb = self.rb.borrow_mut();
        let n = rb.readable().min(buf.len());
        if n > 0 {
            let begin = rb.wrap(rb.read_idx);
            let end = begin + n;
            buf[..n].copy_from_slice(&rb.data.as_slice()[begin..end]);
            rb.read(n);
            rb.wake();
            Poll::Ready(Ok(n))
        } else {
            rb.park(cx.waker());
            Poll::Pending
        }
    }
}

pub struct Writer {
    rb: Rc<RefCell<RingBuffer>>,
}

impl AsyncWrite for Writer {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        let mut rb = self.rb.borrow_mut();
        let n = rb.writeable().min(buf.len());
        if n > 0 {
            let begin = rb.wrap(rb.write_idx);
            let end = begin + n;
            rb.data.as_mut_slice()[begin..end].copy_from_slice(&buf[..n]);
            rb.wrote(n);
            rb.wake();
            Poll::Ready(Ok(n))
        } else {
            rb.park(cx.waker());
            Poll::Pending
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Error>> {
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use proptest_derive::Arbitrary;

    /// Anything you can do to a RingBuffer.
    #[derive(Debug, Arbitrary)]
    enum Operation {
        Write(Vec<u8>), // amount to write
        Read(u8),       // buffer size to use in read
    }

    /// A very simple 'oracle' implementation.
    struct Model {
        capacity: usize,
        data: Vec<u8>,
    }

    impl Model {
        fn new(capacity: usize) -> Model {
            Model {
                capacity,
                data: Vec::new(),
            }
        }

        fn write(&mut self, bytes: &[u8]) -> usize {
            let before = self.data.len();

            self.data.extend_from_slice(&bytes);
            self.data.resize(self.capacity.min(self.data.len()), 0);

            self.data.len() - before
        }

        fn read(&mut self, n: usize) -> Vec<u8> {
            self.data.drain(..n.min(self.data.len())).collect()
        }
    }

    proptest! {
        #[test]
        fn it_works(capacity in any::<u8>(),
                    operations in any::<Vec<Operation>>()) {
            let capacity = capacity as usize;
            let mut model = Model::new(capacity);
            let (mut tx, mut rx) = RingBuffer::with_capacity(capacity);

            for op in operations {
                match op {
                    Operation::Write(data) => {
                        let written = tx.write(&data).now_or_never().unwrap_or(Ok(0)).expect("can't fail");

                        // We might have written less than the full buffer due to wrapping.
                        prop_assert_eq!(model.write(&data[..written]), written);
                    }

                    Operation::Read(n) => {
                        let n = n as usize;
                        let mut buf = [0; 256];
                        let nread = rx.read(&mut buf[..n]).now_or_never().unwrap_or(Ok(0)).expect("can't fail");

                        // We might have read less than the full buffer due to wrapping.
                        let expected = model.read(nread);
                        prop_assert_eq!(expected, &buf[..nread]);
                    }
                }
            }

        }
    }
}
