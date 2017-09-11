// Copyright 2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

/// Stream channels
///
/// This is the flavor of channels which are optimized for one sender and one
/// receiver. The sender will be upgraded to a shared channel if the channel is
/// cloned.
///
/// High level implementation details can be found in the comment of the parent
/// module.

pub use self::Failure::*;
pub use self::UpgradeResult::*;
pub use self::SelectionResult::*;
use self::Message::*;

use std::isize;
use std::marker::PhantomData;
use std::time::Instant;

use std::sync::atomic::{AtomicUsize, Ordering, AtomicBool};
use std::sync::mpsc::Receiver;


use blocking::{self, SignalToken};
use spsc;
use spsc2;

const DISCONNECTED: isize = isize::MIN;
#[cfg(test)]
const MAX_STEALS: isize = 5;
#[cfg(not(test))]
const MAX_STEALS: isize = 1 << 20;

pub trait Queue<T> {
    fn new(bound: usize) -> Self;
    fn push(&self, t: T);
    fn pop(&self) -> Option<T>;
    fn peek(&self) -> Option<&mut T>;
}

impl<T> Queue<T> for spsc::Queue<T, spsc::CacheAligned, spsc::NormalNodeCache> {
    fn new(bound: usize) -> Self {
        unsafe { spsc::Queue::aligned(bound) }
    }

    fn push(&self, t: T) {
        self.push(t)
    }
    fn pop(&self) -> Option<T> {
        self.pop()
    }

    fn peek(&self) -> Option<&mut T> {
        self.peek()
    }
}

impl<T> Queue<T> for spsc::Queue<T, spsc::NoAlign, spsc::NormalNodeCache> {
    fn new(bound: usize) -> Self {
        unsafe { spsc::Queue::new(bound) }
    }

    fn push(&self, t: T) {
        self.push(t)
    }
    fn pop(&self) -> Option<T> {
        self.pop()
    }

    fn peek(&self) -> Option<&mut T> {
        self.peek()
    }
}

impl<T> Queue<T> for spsc::Queue<T, spsc::NoAlign, spsc::NoNodeCache> {
    fn new(_: usize) -> Self {
        unsafe { spsc::Queue::no_cache() }
    }

    fn push(&self, t: T) {
        self.push(t)
    }
    fn pop(&self) -> Option<T> {
        self.pop()
    }

    fn peek(&self) -> Option<&mut T> {
        self.peek()
    }
}

impl<T> Queue<T> for spsc::Queue<T, spsc::CacheAligned, spsc::NoNodeCache> {
    fn new(_: usize) -> Self {
        unsafe { spsc::Queue::aligned_no_cache() }
    }

    fn push(&self, t: T) {
        self.push(t)
    }
    fn pop(&self) -> Option<T> {
        self.pop()
    }

    fn peek(&self) -> Option<&mut T> {
        self.peek()
    }
}

impl<T> Queue<T> for spsc2::Queue<T, spsc2::NoAlign> {
    fn new(bound: usize) -> Self {
        unsafe { spsc2::Queue::new(bound) }
    }

    fn push(&self, t: T) {
        self.push(t)
    }
    fn pop(&self) -> Option<T> {
        self.pop()
    }

    fn peek(&self) -> Option<&mut T> {
        self.peek()
    }
}

impl<T> Queue<T> for spsc2::Queue<T, spsc2::CacheAligned> {
    fn new(bound: usize) -> Self {
        unsafe { spsc2::Queue::aligned(bound) }
    }

    fn push(&self, t: T) {
        self.push(t)
    }
    fn pop(&self) -> Option<T> {
        self.pop()
    }

    fn peek(&self) -> Option<&mut T> {
        self.peek()
    }
}

unsafe impl<Q, T> Send for Packet<Q, T> where Q: Send + Sync, T: Send + Sync {}
unsafe impl<Q, T> Sync for Packet<Q, T> where Q: Send + Sync, T: Send + Sync {}

#[repr(align(64))]
struct AlignToCache;

struct CacheAligned<T>(T, [AlignToCache; 0]);

impl<T> CacheAligned<T> {
     fn new(t: T) -> Self {
         CacheAligned(t, [])
     }
}

impl<T> ::std::ops::Deref for CacheAligned<T> {
     type Target = T;
     fn deref(&self) -> &Self::Target {
         &self.0
     }
}

pub struct Packet<Q, T> {
    queue: Q, // internal queue for all message
    port_dropped: CacheAligned<AtomicBool>, // flag if the channel has been destroyed.
    to_wake: CacheAligned<AtomicUsize>, // SignalToken for the blocked thread to wake up
    _pd: PhantomData<T>,
}

#[derive(Debug)]
pub enum Failure<T> {
    Empty,
    Disconnected,
    Upgraded(Receiver<T>),
}

pub enum UpgradeResult {
    UpSuccess,
    UpDisconnected,
    UpWoke(SignalToken),
}

pub enum SelectionResult<T> {
    SelSuccess,
    SelCanceled,
    SelUpgraded(SignalToken, Receiver<T>),
}

// Any message could contain an "upgrade request" to a new shared port, so the
// internal queue it's a queue of T, but rather Message<T>
pub enum Message<T> {
    Data(T),
    GoUp(Receiver<T>),
}

impl<Q, T> Packet<Q, T>
where Q: Queue<Message<T>> {
    pub fn new() -> Self {
        Packet {
            queue: Q::new(128),

            to_wake: CacheAligned::new(AtomicUsize::new(0)),

            port_dropped: CacheAligned::new(AtomicBool::new(false)),
            _pd: Default::default(),
        }
    }

    pub fn send(&self, t: T) -> Result<(), T> {
        // If the other port has deterministically gone away, then definitely
        // must return the data back up the stack. Otherwise, the data is
        // considered as being sent.
        if self.port_dropped.load(Ordering::SeqCst) { return Err(t) }

        match self.do_send(Data(t)) {
            UpSuccess | UpDisconnected => {},
            UpWoke(token) => { token.signal(); }
        }
        Ok(())
    }

    pub fn upgrade(&self, up: Receiver<T>) -> UpgradeResult {
        // If the port has gone away, then there's no need to proceed any
        // further.
        if self.port_dropped.load(Ordering::SeqCst) { return UpDisconnected }

        self.do_send(GoUp(up))
    }

    fn do_send(&self, t: Message<T>) -> UpgradeResult {
        self.queue.push(t);
        //TODO DISCONNECTED?
        if self.port_dropped.load(Ordering::SeqCst) {
            // Be sure to preserve the disconnected state, and the return value
            // in this case is going to be whether our data was received or not.
            // This manifests itself on whether we have an empty queue or not.
            //
            // Primarily, are required to drain the queue here because the port
            // will never remove this data. We can only have at most one item to
            // drain (the port drains the rest).
            let first = self.queue.pop();
            let second = self.queue.pop();
            assert!(second.is_none());

            return match first {
                Some(..) => UpSuccess,  // we failed to send the data
                None => UpDisconnected, // we successfully sent data
            }
        }

        match self.try_take_to_wake() {
            Some(token) => UpWoke(token),
            None => UpSuccess,
        }
    }

    // Consumes ownership of the 'to_wake' field.
    fn take_to_wake(&self) -> SignalToken {
        let ptr = self.to_wake.load(Ordering::SeqCst);
        self.to_wake.store(0, Ordering::SeqCst);
        assert!(ptr != 0);
        unsafe { SignalToken::cast_from_usize(ptr) }
    }

    // Consumes ownership of the 'to_wake' field.
    fn try_take_to_wake(&self) -> Option<SignalToken> {
        let ptr = self.to_wake.swap(0, Ordering::SeqCst);
        if ptr == 0 {
            None
        } else {
            Some(unsafe { SignalToken::cast_from_usize(ptr) })
        }
    }

    // Decrements the count on the channel for a sleeper, returning the sleeper
    // back if it shouldn't sleep. Note that this is the location where we take
    // steals into account.
    fn decrement(&self, token: SignalToken) -> Result<Option<T>, SignalToken> {
        assert_eq!(self.to_wake.load(Ordering::SeqCst), 0);
        let ptr = unsafe { token.cast_to_usize() };
        self.to_wake.store(ptr, Ordering::SeqCst);

        match self.try_recv() {
            Err(Empty) | Err(Disconnected) => {}
            Err(Upgraded(..)) => unimplemented!(),
            Ok(data) => {
                self.to_wake.store(0, Ordering::SeqCst);
                return Ok(Some(data))
            }
        }

        if self.port_dropped.load(Ordering::SeqCst) {
            self.to_wake.store(0, Ordering::SeqCst);
            return Err(unsafe { SignalToken::cast_from_usize(ptr) })
        }

        return Ok(None)
    }

    pub fn recv(&self, deadline: Option<Instant>) -> Result<T, Failure<T>> {
        // Optimistic preflight check (scheduling is expensive).
        match self.try_recv() {
            Err(Empty) => {}
            data => return data,
        }
        'recv: loop {
            // Welp, our channel has no data. Deschedule the current thread and
            // initiate the blocking protocol.
            let (wait_token, signal_token) = blocking::tokens();
            match self.decrement(signal_token) {
                Ok(Some(data)) => return Ok(data),
                Ok(None) => if let Some(deadline) = deadline {
                        wait_token.wait_max_until(deadline);
                    } else {
                        wait_token.wait();
                    },
                Err(..) => {}
            }

            match self.try_recv() {
                // We get get spurious wakeups under the correct interleaving
                // so if we recv an Empty here go back to sleep
                Err(Empty) => continue 'recv,
                // Messages which actually popped from the queue shouldn't count as
                // a steal, so offset the decrement here (we already have our
                // "steal" factored into the channel count above).
                data @ Ok(..) | data @ Err(Upgraded(..)) | data @ Err(Disconnected) => return data,
            }
        }
    }

    pub fn try_recv(&self) -> Result<T, Failure<T>> {
        match self.queue.pop() {
            Some(data) => {
                match data {
                    Data(t) => Ok(t),
                    GoUp(up) => Err(Upgraded(up)),
                }
            },

            None => {
                if !self.port_dropped.load(Ordering::SeqCst) {
                    return Err(Empty)
                }
                match self.queue.pop() {
                    Some(Data(t)) => Ok(t),
                    Some(GoUp(up)) => Err(Upgraded(up)),
                    None => Err(Disconnected),
                }
            }
        }
    }

    // drops the a sender
    pub fn drop_chan(&self) {
        // Dropping a channel is pretty simple, we just flag it as disconnected
        // and then wakeup a blocker if there is one.
        self.port_dropped.store(true, Ordering::SeqCst);
        if let Some(to_wake) = self.try_take_to_wake() {
            to_wake.signal();
        }
    }

    // drops the one receiver
    // FIXME: The simplest way to implement this without a count is likely 2-phase commit:
    //        1. mark the receiver as dropped, after this no new sends can start
    //        2. wait for sender to not be sendning
    //        3. flush any remaining
    pub fn drop_port(&self) {
        // Dropping a port seems like a fairly trivial thing. In theory all we
        // need to do is flag that we're disconnected and then everything else
        // can take over (we don't have anyone to wake up).
        //
        // The catch for Ports is that we want to drop the entire contents of
        // the queue. There are multiple reasons for having this property, the
        // largest of which is that if another chan is waiting in this channel
        // (but not received yet), then waiting on that port will cause a
        // deadlock.
        //
        // So if we accept that we must now destroy the entire contents of the
        // queue, this code may make a bit more sense. The tricky part is that
        // we can't let any in-flight sends go un-dropped, we have to make sure
        // *everything* is dropped and nothing new will come onto the channel.

        // The first thing we do is set a flag saying that we're done for. All
        // sends are gated on this flag, so we're immediately guaranteed that
        // there are a bounded number of active sends that we'll have to deal
        // with.
        self.port_dropped.store(true, Ordering::SeqCst);

        // Now that we're guaranteed to deal with a bounded number of senders,
        // we need to drain the queue. This draining process happens atomically
        // with respect to the "count" of the channel. If the count is nonzero
        // (with steals taken into account), then there must be data on the
        // channel. In this case we drain everything and then try again. We will
        // continue to fail while active senders send data while we're dropping
        // data, but eventually we're guaranteed to break out of this loop
        // (because there is a bounded number of senders).

        //TODO we need a second signal to indicate that the sender will no longer send
        //     this can be easily done with an additional read-mostly flag
        while let Some(_) = self.queue.pop() { }

        // At this point in time, we have gated all future senders from sending,
        // and we have flagged the channel as being disconnected. The senders
        // still have some responsibility, however, because some sends may not
        // complete until after we flag the disconnection. There are more
        // details in the sending methods that see DISCONNECTED
    }
}

impl<Q, T> Packet<Q, T> {
    fn drop(&mut self) {
        // Note that this load is not only an assert for correctness about
        // disconnection, but also a proper fence before the read of
        // `to_wake`, so this assert cannot be removed with also removing
        // the `to_wake` assert.
        // assert_eq!(self.cnt.load(Ordering::SeqCst), DISCONNECTED);
        assert_eq!(self.to_wake.load(Ordering::SeqCst), 0);
    }
}