# std_spsc_is_slow

As of `rustc 1.20.0 (f3d6973f4 2017-08-27)` the standard library's
[`mpsc`](https://doc.rust-lang.org/1.20.0/std/sync/mpsc/index.html)
has an optimization for the case where there is exactly 1 sender and 1
receiver. Unfortunately, in currently this optimized version seems to
be slower than the regular multi producer code. This repository contains
investigations into that slowdown.

## Basic Benchmark

The basic benchmark, based on one from
[crossbeam](https://github.com/crossbeam-rs/crossbeam/blob/2b03f1fd8c6dd5a045f3b77a0d2bbebff4776ade/src/bin/bench.rs)
is:

```rust
fn bench_mpsc_stream() -> f64 {
    let (sender, reciever) = channel();
    bench_spsc(sender, reciever)
}

fn bench_mpsc_shared() -> f64 {
    let (sender, reciever) = channel();
    // this clone forces the queue into shared mode and makes the benchmark faster
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
```

It can be run on stable with `cargo run --release`, and, on a recent intel machine,
typically has an output like
```
spsc stream        185 ns/send
spsc shared        112 ns/send
```

## Other Investigations

The repo also contains investigations into what may be causing this slowdown,
along with other explorations of mpsc performance that occurred along the way,
currently focusing on the underlying datastructures.

These benchmarks can be run with `cargo +nightly run --release --features "queue_experiments"` and a the results from a typical run are:
```
spsc stream        201 ns/send
spsc shared        134 ns/send
----
mpmc baseline       59 ns/send
aligned             46 ns/send
----
spsc baseline       75 ns/send
bigger cache        67 ns/send
aligned             97 ns/send
unbounded           30 ns/send
no cache            46 ns/send
unbounded, aligned  12 ns/send
no cache, aligned   47 ns/send
----
less contention spsc  35 ns/send
aligned                9 ns/send
aligned, size =    1   9 ns/send
aligned, size =    8  10 ns/send
aligned, size =   16   9 ns/send
aligned, size =   32  11 ns/send
aligned, size =   64  10 ns/send
aligned, size =  128  13 ns/send
aligned, size =  256  12 ns/send
aligned, size =  512  13 ns/send
aligned, size = 1024   9 ns/send
----
stream baseline      205 ns/send
aligned              212 ns/send
no cache             103 ns/send
aligned, no cache     91 ns/send
less contend         179 ns/send
less contend aligned 102 ns/send
----
stream2 baseline     131 ns/send
aligned              112 ns/send
no cache              91 ns/send
aligned, no cache     58 ns/send
less contend         103 ns/send
less contend aligned  27 ns/send
```
From this I draw the following tentative conclusions:

1. There is false sharing happening in mpsc_queue.
2. The current fixed-size node cache in spsc_queue seems to be a pessimization,
mainly due to contention over the counters.
A version which only keeps counters on the consumer side only (shown in the last set above) can perform on par with an ubounded cache.
3. False sharing may also become an issue for spsc_queue, but it is currently hidden by other overheads.
4. Datastructure differences do not seem to account for all the difference in performance between shared and stream mode, though we would need more extensive benchmarking to really tell.
