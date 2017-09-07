//! As of rustc 1.20.0 (f3d6973f4 2017-08-27) std::sync::mpsc is faster
//! when used in shared mode than the its internal spsc stream mode.
//! Note: this is partly disappears on 1.22.0-nightly (981ce7d8d 2017-09-03)
//!       due to shared getting slower
//!
//! aws c4.2xlarge
//! stable rustc 1.20.0 (f3d6973f4 2017-08-27)
//! spsc stream      127.5086168
//! spsc shared      66.2942756
//!
//! nightly rustc 1.22.0-nightly (981ce7d8d 2017-09-03)
//! spsc stream      111.13214775
//! spsc shared      61.84188775
//! just queue       34.37997215
//! aligned queue    43.47179525
//! unbounded queue  29.22656035
//! no cache queue   40.32926815
//! u/a queue        23.9909434
//! n/a              39.4851343
//!
//!
#![cfg_attr(feature = "queue_experiments", feature(repr_align, attr_literals, box_syntax, test))]
#![allow(dead_code)]

// based on crossbeam's bin/bench

//using crossbeam for scoped threads
extern crate crossbeam;

#[cfg(feature="queue_experiments")]
extern crate test;

use crossbeam::scope;

#[cfg(feature="queue_experiments")]
use std::sync::Arc;
use std::sync::mpsc::{channel, Sender, Receiver};
use std::time::Duration;

#[cfg(feature="queue_experiments")]
use test::black_box;

#[cfg(not(feature="queue_experiments"))]
fn black_box<T>(t: T) -> T { t }


// A copy of libstd/sync/mpsc/spsc_queue.rs to test various optimazations on
#[cfg(feature="queue_experiments")]
mod spsc;

// A copy of libstd/sync/mpsc/mpsc_queue.rs to compare with spsc
// the effects of false sharing
#[cfg(feature="queue_experiments")]
mod mpmc;

// A version of spsc where all infmation on chache size is maintained exclusively by the consumer
#[cfg(feature="queue_experiments")]
mod spsc2;

fn main() {
    println!("spsc stream        {:>3.0} ns/send", bench_mpsc_stream());
    println!("spsc shared        {:>3.0} ns/send", bench_mpsc_shared());

    #[cfg(feature="queue_experiments")]
    unsafe {
        println!("----");
        println!("mpmc baseline      {:>3.0} ns/send", bench_mpmc_queue(mpmc::Queue::new()));
        println!("aligned            {:>3.0} ns/send", bench_mpmc_queue(mpmc::Queue::aligned()));
        println!("----");
        println!("spsc baseline      {:>3.0} ns/send", bench_spsc_queue(spsc::Queue::new(128)));
        println!("bigger cache       {:>3.0} ns/send", bench_spsc_queue(spsc::Queue::new(1024)));
        println!("aligned            {:>3.0} ns/send", bench_spsc_queue(spsc::Queue::aligned(128)));
        println!("unbounded          {:>3.0} ns/send", bench_spsc_queue(spsc::Queue::new(0)));
        println!("no cache           {:>3.0} ns/send", bench_spsc_queue(spsc::Queue::no_cache()));
        println!("unbounded, aligned {:>3.0} ns/send", bench_spsc_queue(spsc::Queue::aligned(0)));
        println!("no cache, aligned  {:>3.0} ns/send", bench_spsc_queue(spsc::Queue::aligned_no_cache()));
        println!("----");
        println!("less contention spsc {:>3.0} ns/send", bench_spsc2_queue(spsc2::Queue::new(128)));
        println!("alinged              {:>3.0} ns/send", bench_spsc2_queue(spsc2::Queue::aligned(128)));
        println!("aligned, size =    1 {:>3.0} ns/send", bench_spsc2_queue(spsc2::Queue::aligned(1)));
        println!("aligned, size =    8 {:>3.0} ns/send", bench_spsc2_queue(spsc2::Queue::aligned(8)));
        println!("aligned, size =   16 {:>3.0} ns/send", bench_spsc2_queue(spsc2::Queue::aligned(16)));
        println!("aligned, size =   32 {:>3.0} ns/send", bench_spsc2_queue(spsc2::Queue::aligned(32)));
        println!("aligned, size =   64 {:>3.0} ns/send", bench_spsc2_queue(spsc2::Queue::aligned(64)));
        println!("aligned, size =  128 {:>3.0} ns/send", bench_spsc2_queue(spsc2::Queue::aligned(128)));
        println!("aligned, size =  256 {:>3.0} ns/send", bench_spsc2_queue(spsc2::Queue::aligned(256)));
        println!("aligned, size =  512 {:>3.0} ns/send", bench_spsc2_queue(spsc2::Queue::aligned(512)));
        println!("aligned, size = 1024 {:>3.0} ns/send", bench_spsc2_queue(spsc2::Queue::aligned(1024)));
    }

}

fn bench_mpsc_stream() -> f64 {
    let (sender, reciever) = channel();
    bench_spsc(sender, reciever)
}

fn bench_mpsc_shared() -> f64 {
    let (sender, reciever) = channel();
    // this clone make the benchmark faster
    let _clone = sender.clone();
    bench_spsc(sender, reciever)
}

const COUNT: u64 = 10_000_000;

fn bench_spsc(tx: Sender<u64>, rx: Receiver<u64>) -> f64 {
    // ensure that the channel is not in Once mode
    tx.send(0).unwrap();
    tx.send(0).unwrap();
    rx.recv().unwrap();
    rx.recv().unwrap();

    let start = ::std::time::Instant::now();
    scope(|scope| {
        scope.spawn(move || {
            for x in 0..(COUNT*2) {
                let _ = black_box(tx.send(x));
            }
        });

        for _i in 0..(COUNT*2) {
            let _ = black_box(rx.recv().unwrap());
        }
    });
    let d = start.elapsed();

    nanos(d) / ((COUNT*2) as f64)
}

#[cfg(feature="queue_experiments")]
fn bench_spsc_queue<A, C>(queue: spsc::Queue<u64, A, C>) -> f64
where C : spsc::UseCache {
    let tx = Arc::new(queue);
    let rx = tx.clone();
    let start = ::std::time::Instant::now();
    scope(|scope| {
        scope.spawn(move || {
            for x in 0..(COUNT*2) {
                let _ = black_box(tx.push(x));
            }
        });

        for _i in 0..(COUNT*2) {
            while let None = black_box(rx.pop()) {}
        }
    });
    let d = start.elapsed();

    nanos(d) / ((COUNT*2) as f64)
}

#[cfg(feature="queue_experiments")]
fn bench_spsc2_queue<A>(queue: spsc2::Queue<u64, A>) -> f64 {
    let tx = Arc::new(queue);
    let rx = tx.clone();
    let start = ::std::time::Instant::now();
    scope(|scope| {
        scope.spawn(move || {
            for x in 0..(COUNT*2) {
                let _ = black_box(tx.push(x));
            }
        });

        for _i in 0..(COUNT*2) {
            while let None = black_box(rx.pop()) {}
        }
    });
    let d = start.elapsed();

    nanos(d) / ((COUNT*2) as f64)
}

#[cfg(feature="queue_experiments")]
fn bench_mpmc_queue<Align>(queue: mpmc::Queue<u64, Align>) -> f64 {
    let tx = Arc::new(queue);
    let rx = tx.clone();
    let start = ::std::time::Instant::now();
    scope(|scope| {
        scope.spawn(move || {
            for x in 0..(COUNT*2) {
                let _ = black_box(tx.push(x));
            }
        });

        for _i in 0..(COUNT*2) {
            loop {
                match black_box(rx.pop()) {
                    mpmc::Data(..) => break,
                    _ => continue,
                }
            }
        }
    });
    let d = start.elapsed();

    nanos(d) / ((COUNT*2) as f64)
}

fn nanos(d: Duration) -> f64 {
    d.as_secs() as f64 * 1000000000f64 + (d.subsec_nanos() as f64)
}

#[cfg(feature="queue_experiments")]
mod bench {
    #![allow(non_snake_case)]

    use test::{Bencher, black_box};

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use crossbeam::scope;

    use ::{spsc, mpmc};

    #[bench]
    fn mpmc_base_send(b: &mut Bencher) {
        bench_mpmc_queue(mpmc::Queue::new(), b)
    }

    #[bench]
    fn mpmc_alinged_send(b: &mut Bencher) {
        bench_mpmc_queue(mpmc::Queue::aligned(), b)
    }

    #[bench]
    fn spsc_base_send(b: &mut Bencher) {
        unsafe { bench_spsc_queue(spsc::Queue::new(128), b) }
    }

    #[bench]
    fn spsc_aligned_send(b: &mut Bencher) {
        unsafe { bench_spsc_queue(spsc::Queue::aligned(128), b) }
    }

    #[bench]
    fn spsc_unbounded_send(b: &mut Bencher) {
        unsafe { bench_spsc_queue(spsc::Queue::new(0), b) }
    }

    #[bench]
    fn spsc__no_cache__send(b: &mut Bencher) {
        unsafe { bench_spsc_queue(spsc::Queue::no_cache(), b) }
    }

    #[bench]
    fn spsc_unbounded_aligned_send(b: &mut Bencher) {
        unsafe { bench_spsc_queue(spsc::Queue::aligned(0), b) }
    }

    #[bench]
    fn spsc__no_cache__aligned_send(b: &mut Bencher) {
        unsafe { bench_spsc_queue(spsc::Queue::aligned_no_cache(), b) }
    }

    fn bench_spsc_queue<A, C>(queue: spsc::Queue<u64, A, C>, b: &mut Bencher)
    where C: spsc::UseCache {
        let tx = Arc::new(queue);
        let rx = tx.clone();
        let done = AtomicBool::new(false);
        scope(|scope| {
            let done = &done;
            scope.spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    let _ = rx.pop();
                }
            });

            let mut x = 0;
            b.iter(|| { black_box(tx.push(x)); x += 1 });
            done.store(true, Ordering::Relaxed);
        });
    }

    fn bench_mpmc_queue<A>(queue: mpmc::Queue<u64, A>, b: &mut Bencher) {
        let tx = Arc::new(queue);
        let rx = tx.clone();
        let done = AtomicBool::new(false);
        scope(|scope| {
            let done = &done;
            scope.spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    let _ = rx.pop();
                }
            });

            let mut x = 0;
            b.iter(|| { black_box(tx.push(x)); x += 1 });
            done.store(true, Ordering::Relaxed);
        });
    }
}
