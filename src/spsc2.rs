//! experiments in different optimizations of std's spsc implementation.
//! namely:
//!   - cache aligning the producer and consumer
//!   - unbounding the node cache
//!   - removing the node cache entirely

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::ptr;

struct Node<T> {
    // FIXME: this could be an uninitialized T if we're careful enough, and
    //      that would reduce memory usage (and be a bit faster).
    //      is it worth it?
    value: Option<T>,           // nullable for re-use of nodes
    cached: bool,
    next: AtomicPtr<Node<T>>,   // next node in the queue
}

pub struct NoAlign;

#[repr(align(64))]
pub struct CacheAligned;

pub struct Queue<T, Align> {
    // consumer fields
    consumer: Consumer<T, Align>,

    // producer fields
    producer: Producer<T, Align>,
}

struct Consumer<T, Align> {
    tail: UnsafeCell<*mut Node<T>>, // where to pop from
    tail_prev: AtomicPtr<Node<T>>, // where to pop from
    cache_bound: usize, // maximum cache size
    cached_nodes: AtomicUsize, // number of nodes marked as cachable
    _align: [Align; 0],
}

struct Producer<T, Align> {
    head: UnsafeCell<*mut Node<T>>,      // where to push to
    first: UnsafeCell<*mut Node<T>>,     // where to get new nodes from
    tail_copy: UnsafeCell<*mut Node<T>>, // between first/tail
    _align: [Align; 0],
}

unsafe impl<T: Send, A> Send for Queue<T, A> { }
unsafe impl<T: Send, A> Sync for Queue<T, A> { }

pub type _Queue<T> = Queue<T, NoAlign>;
pub type AQueue<T> = Queue<T, CacheAligned>;

pub struct NormalNodeCache;
pub struct NoNodeCache;

impl<T> Node<T> {
    fn new() -> *mut Node<T> {
        Box::into_raw(box Node {
            value: None,
            cached: false,
            next: AtomicPtr::new(ptr::null_mut::<Node<T>>()),
        })
    }
}

impl<T> Queue<T, NoAlign> {
    /// Creates a new queue.
    ///
    /// This is unsafe as the type system doesn't enforce a single
    /// consumer-producer relationship. It also allows the consumer to `pop`
    /// items while there is a `peek` active due to all methods having a
    /// non-mutable receiver.
    ///
    /// # Arguments
    ///
    ///   * `bound` - This queue implementation is implemented with a linked
    ///               list, and this means that a push is always a malloc. In
    ///               order to amortize this cost, an internal cache of nodes is
    ///               maintained to prevent a malloc from always being
    ///               necessary. This bound is the limit on the size of the
    ///               cache (if desired). If the value is 0, then the cache has
    ///               no bound. Otherwise, the cache will never grow larger than
    ///               `bound` (although the queue itself could be much larger.
    pub unsafe fn new(bound: usize) -> Self {
        let n1 = Node::new();
        let n2 = Node::new();
        (*n1).next.store(n2, Ordering::Relaxed);
        Queue {
            consumer: Consumer {
                tail: UnsafeCell::new(n2),
                tail_prev: AtomicPtr::new(n1),
                cache_bound: bound,
                cached_nodes: AtomicUsize::new(0),
                _align: [],
            },
            producer: Producer {
                head: UnsafeCell::new(n2),
                first: UnsafeCell::new(n1),
                tail_copy: UnsafeCell::new(n1),
                _align: [],
            },
        }
    }
}

impl<T> Queue<T, CacheAligned> {
    pub unsafe fn aligned(bound: usize) -> Self {
        let n1 = Node::new();
        let n2 = Node::new();
        (*n1).next.store(n2, Ordering::Relaxed);
        Queue {
            consumer: Consumer {
                tail: UnsafeCell::new(n2),
                tail_prev: AtomicPtr::new(n1),
                cache_bound: bound,
                cached_nodes: AtomicUsize::new(0),
                _align: [],
            },
            producer: Producer {
                head: UnsafeCell::new(n2),
                first: UnsafeCell::new(n1),
                tail_copy: UnsafeCell::new(n1),
                _align: [],
            },
        }
    }
}

impl<T, Align> Queue<T, Align> {


    /// Pushes a new value onto this queue. Note that to use this function
    /// safely, it must be externally guaranteed that there is only one pusher.
    pub fn push(&self, t: T) {
        unsafe {
            // Acquire a node (which either uses a cached one or allocates a new
            // one), and then append this to the 'head' node.
            let n = self.alloc();
            assert!((*n).value.is_none());
            (*n).value = Some(t);
            (*n).next.store(ptr::null_mut(), Ordering::Relaxed);
            (**self.producer.head.get()).next.store(n, Ordering::Release);
            *self.producer.head.get() = n;
        }
    }

    unsafe fn alloc(&self) -> *mut Node<T> {
        // First try to see if we can consume the 'first' node for our uses.
        // We try to avoid as many atomic instructions as possible here, so
        // the addition to cache_subtractions is not atomic (plus we're the
        // only one subtracting from the cache).
        if *self.producer.first.get() != *self.producer.tail_copy.get() {
            let ret = *self.producer.first.get();
            *self.producer.first.get() = (*ret).next.load(Ordering::Relaxed);
            return ret;
        }
        // If the above fails, then update our copy of the tail and try
        // again.
        *self.producer.tail_copy.get() = self.consumer.tail_prev.load(Ordering::Acquire);
        if *self.producer.first.get() != *self.producer.tail_copy.get() {
            let ret = *self.producer.first.get();
            *self.producer.first.get() = (*ret).next.load(Ordering::Relaxed);
            return ret;
        }
        // If all of that fails, then we have to allocate a new node
        // (there's nothing in the node cache).
        Node::new()
    }

    /// Attempts to pop a value from this queue. Remember that to use this type
    /// safely you must ensure that there is only one popper at a time.
    pub fn pop(&self) -> Option<T> {
        unsafe {
            // The `tail` node is not actually a used node, but rather a
            // sentinel from where we should start popping from. Hence, look at
            // tail's next field and see if we can use it. If we do a pop, then
            // the current tail node is a candidate for going into the cache.
            let tail = *self.consumer.tail.get();
            let next = (*tail).next.load(Ordering::Acquire);
            if next.is_null() { return None }
            assert!((*next).value.is_some());
            let ret = (*next).value.take();

            *self.consumer.tail.get() = next;

            if self.consumer.cache_bound == 0 {
                self.consumer.tail_prev.store(tail, Ordering::Release);
            } else {
                let cached_nodes = self.consumer.cached_nodes.load(Ordering::Relaxed);
                if cached_nodes < self.consumer.cache_bound && !(*tail).cached {
                    self.consumer.cached_nodes.store(cached_nodes, Ordering::Relaxed);
                    (*tail).cached = true;
                }

                if (*tail).cached {
                    self.consumer.tail_prev.store(tail, Ordering::Release);
                } else {
                    (*self.consumer.tail_prev.load(Ordering::Relaxed))
                          .next.store(next, Ordering::Relaxed);
                    // We have successfully erased all references to 'tail', so
                    // now we can safely drop it.
                    let _: Box<Node<T>> = Box::from_raw(tail);
                }
            }
            ret
        }
    }

    /// Attempts to peek at the head of the queue, returning `None` if the queue
    /// has no data currently
    ///
    /// # Warning
    /// The reference returned is invalid if it is not used before the consumer
    /// pops the value off the queue. If the producer then pushes another value
    /// onto the queue, it will overwrite the value pointed to by the reference.
    pub fn peek(&self) -> Option<&mut T> {
        // This is essentially the same as above with all the popping bits
        // stripped out.
        unsafe {
            let tail = *self.consumer.tail.get();
            let next = (*tail).next.load(Ordering::Acquire);
            if next.is_null() { None } else { (*next).value.as_mut() }
        }
    }
}

impl<T, Align> Drop for Queue<T, Align> {
    fn drop(&mut self) {
        unsafe {
            let mut cur = *self.producer.first.get();
            while !cur.is_null() {
                let next = (*cur).next.load(Ordering::Relaxed);
                let _n: Box<Node<T>> = Box::from_raw(cur);
                cur = next;
            }
        }
    }
}

#[cfg(all(test, not(target_os = "emscripten")))]
mod tests {
    use std::sync::Arc;
    use super::Queue;
    use std::thread;
    use std::sync::mpsc::channel;

    #[test]
    fn smoke() {
        unsafe {
            let queue = Queue::new(0);
            queue.push(1);
            queue.push(2);
            assert_eq!(queue.pop(), Some(1));
            assert_eq!(queue.pop(), Some(2));
            assert_eq!(queue.pop(), None);
            queue.push(3);
            queue.push(4);
            assert_eq!(queue.pop(), Some(3));
            assert_eq!(queue.pop(), Some(4));
            assert_eq!(queue.pop(), None);
        }
    }

    #[test]
    fn peek() {
        unsafe {
            let queue = Queue::new(0);
            queue.push(vec![1]);

            // Ensure the borrowchecker works
            match queue.peek() {
                Some(vec) => {
                    assert_eq!(&*vec, &[1]);
                },
                None => unreachable!()
            }

            match queue.pop() {
                Some(vec) => {
                    assert_eq!(&*vec, &[1]);
                },
                None => unreachable!()
            }
        }
    }

    #[test]
    fn drop_full() {
        unsafe {
            let q: Queue<Box<_>, _> = Queue::new(0);
            q.push(box 1);
            q.push(box 2);
        }
    }

    #[test]
    fn smoke_bound() {
        unsafe {
            let q = Queue::new(0);
            q.push(1);
            q.push(2);
            assert_eq!(q.pop(), Some(1));
            assert_eq!(q.pop(), Some(2));
            assert_eq!(q.pop(), None);
            q.push(3);
            q.push(4);
            assert_eq!(q.pop(), Some(3));
            assert_eq!(q.pop(), Some(4));
            assert_eq!(q.pop(), None);
        }
    }

    #[test]
    fn stress() {
        unsafe {
            stress_bound(0);
            stress_bound(1);
        }

        unsafe fn stress_bound(bound: usize) {
            let q = Arc::new(Queue::new(bound));

            let (tx, rx) = channel();
            let q2 = q.clone();
            let _t = thread::spawn(move|| {
                for _ in 0..100000 {
                    loop {
                        match q2.pop() {
                            Some(1) => break,
                            Some(_) => panic!(),
                            None => {}
                        }
                    }
                }
                tx.send(()).unwrap();
            });
            for _ in 0..100000 {
                q.push(1);
            }
            rx.recv().unwrap();
        }
    }

    #[test]
    fn stress2() {
        unsafe {
            stress_bound(0);
            stress_bound(1);
        }

        unsafe fn stress_bound(bound: usize) {
            let q = Arc::new(Queue::new(bound));

            let (tx, rx) = channel();
            let q2 = q.clone();
            let _t = thread::spawn(move|| {
                for i in 0..100000 {
                    loop {
                        match q2.pop() {
                            Some(j) => assert_eq!(i, j),
                            None => {}
                        }
                    }
                }
                tx.send(()).unwrap();
            });
            for i in 0..100000 {
                q.push(i);
            }
            rx.recv().unwrap();
        }
    }
}
