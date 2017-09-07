//! std's mpmc datastructure for comparison with spsc.

pub use self::PopResult::*;

use std::ptr;
use std::cell::UnsafeCell;

use std::sync::atomic::{AtomicPtr, Ordering};

/// A result of the `pop` function.
pub enum PopResult<T> {
    /// Some data has been popped
    Data(T),
    /// The queue is empty
    Empty,
    /// The queue is in an inconsistent state. Popping data should succeed, but
    /// some pushers have yet to make enough progress in order allow a pop to
    /// succeed. It is recommended that a pop() occur "in the near future" in
    /// order to see if the sender has made progress or not
    Inconsistent,
}

struct Node<T> {
    next: AtomicPtr<Node<T>>,
    value: Option<T>,
}

struct AlignedPtr<T, Align>(UnsafeCell<*mut Node<T>>, [Align; 0]);

pub struct NoAlign;

#[repr(align(64))]
pub struct CacheAligned;

/// The multi-producer single-consumer structure. This is not cloneable, but it
/// may be safely shared so long as it is guaranteed that there is only one
/// popper at a time (many pushers are allowed).
pub struct Queue<T, Align> {
    head: AtomicPtr<Node<T>>,

    tail: AlignedPtr<T, Align>,
}

unsafe impl<T: Send, Align> Send for Queue<T, Align> { }
unsafe impl<T: Send, Align> Sync for Queue<T, Align> { }

impl<T> Node<T> {
    unsafe fn new(v: Option<T>) -> *mut Node<T> {
        Box::into_raw(box Node {
            next: AtomicPtr::new(ptr::null_mut()),
            value: v,
        })
    }
}

impl<T> Queue<T, NoAlign> {
    /// Creates a new queue that is safe to share among multiple producers and
    /// one consumer.
    pub fn new() -> Self {
        let stub = unsafe { Node::new(None) };
        Queue {
            head: AtomicPtr::new(stub),
            tail: AlignedPtr(UnsafeCell::new(stub), []),
        }
    }
}

impl<T> Queue<T, CacheAligned> {
    pub fn aligned() -> Self {
        let stub = unsafe { Node::new(None) };
        Queue {
            head: AtomicPtr::new(stub),
            tail: AlignedPtr(UnsafeCell::new(stub), []),
        }
    }
}

impl<T, Align> Queue<T, Align> {

    /// Pushes a new value onto this queue.
    pub fn push(&self, t: T) {
        unsafe {
            let n = Node::new(Some(t));
            let prev = self.head.swap(n, Ordering::AcqRel);
            (*prev).next.store(n, Ordering::Release);
        }
    }

    /// Pops some data from this queue.
    ///
    /// Note that the current implementation means that this function cannot
    /// return `Option<T>`. It is possible for this queue to be in an
    /// inconsistent state where many pushes have succeeded and completely
    /// finished, but pops cannot return `Some(t)`. This inconsistent state
    /// happens when a pusher is pre-empted at an inopportune moment.
    ///
    /// This inconsistent state means that this queue does indeed have data, but
    /// it does not currently have access to it at this time.
    pub fn pop(&self) -> PopResult<T> {
        unsafe {
            let tail = *self.tail.0.get();
            let next = (*tail).next.load(Ordering::Acquire);

            if !next.is_null() {
                *self.tail.0.get() = next;
                assert!((*tail).value.is_none());
                assert!((*next).value.is_some());
                let ret = (*next).value.take().unwrap();
                let _: Box<Node<T>> = Box::from_raw(tail);
                return Data(ret);
            }

            if self.head.load(Ordering::Acquire) == tail {Empty} else {Inconsistent}
        }
    }
}

impl<T, Align> Drop for Queue<T, Align> {
    fn drop(&mut self) {
        unsafe {
            let mut cur = *self.tail.0.get();
            while !cur.is_null() {
                let next = (*cur).next.load(Ordering::Relaxed);
                let _: Box<Node<T>> = Box::from_raw(cur);
                cur = next;
            }
        }
    }
}

#[cfg(all(test, not(target_os = "emscripten")))]
mod tests {
    use std::sync::mpsc::channel;
    use super::{Queue, Data, Empty, Inconsistent};
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_full() {
        let q: Queue<Box<_>, _> = Queue::new();
        q.push(box 1);
        q.push(box 2);
    }

    #[test]
    fn test() {
        let nthreads = 8;
        let nmsgs = 1000;
        let q = Queue::new();
        match q.pop() {
            Empty => {}
            Inconsistent | Data(..) => panic!()
        }
        let (tx, rx) = channel();
        let q = Arc::new(q);

        for _ in 0..nthreads {
            let tx = tx.clone();
            let q = q.clone();
            thread::spawn(move|| {
                for i in 0..nmsgs {
                    q.push(i);
                }
                tx.send(()).unwrap();
            });
        }

        let mut i = 0;
        while i < nthreads * nmsgs {
            match q.pop() {
                Empty | Inconsistent => {},
                Data(_) => { i += 1 }
            }
        }
        drop(tx);
        for _ in 0..nthreads {
            rx.recv().unwrap();
        }
    }
}
