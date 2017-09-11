```
Architecture:          armv7l
Byte Order:            Little Endian
CPU(s):                4
On-line CPU(s) list:   0-3
Thread(s) per core:    1
Core(s) per socket:    4
Socket(s):             1
Model name:            ARMv7 Processor rev 4 (v7l)
CPU max MHz:           1200.0000
CPU min MHz:           600.0000
```

default settings:
```
spsc stream        316 ns/send
spsc shared        528 ns/send
----
mpmc baseline      442 ns/send
aligned            440 ns/send
----
spsc baseline      185 ns/send
bigger cache       182 ns/send
aligned             91 ns/send
unbounded          123 ns/send
no cache           454 ns/send
unbounded, aligned  91 ns/send
no cache, aligned  450 ns/send
----
less contention spsc 121 ns/send
aligned              104 ns/send
aligned, size =    1 105 ns/send
aligned, size =    8 105 ns/send
aligned, size =   16 104 ns/send
aligned, size =   32 105 ns/send
aligned, size =   64 104 ns/send
aligned, size =  128 104 ns/send
aligned, size =  256 104 ns/send
aligned, size =  512 105 ns/send
aligned, size = 1024 104 ns/send
----
stream baseline      403 ns/send
aligned              383 ns/send
no cache             673 ns/send
aligned, no cache    653 ns/send
less contend         277 ns/send
less contend aligned 263 ns/send
----
stream2 baseline     396 ns/send
aligned              369 ns/send
no cache             655 ns/send
aligned, no cache    643 ns/send
less contend         265 ns/send
less contend aligned 244 ns/send
```

using set-affinity to force threads on separate cores:
```
spsc stream        319 ns/send  //XXX this data point cannot be compared to the others, see the rerun at the bottom
spsc shared        862 ns/send
----
mpmc baseline      732 ns/send
aligned            733 ns/send
----
spsc baseline      190 ns/send
bigger cache       732 ns/send
aligned            750 ns/send
unbounded           90 ns/send
no cache           729 ns/send
unbounded, aligned  86 ns/send
no cache, aligned  732 ns/send
----
less contention spsc 122 ns/send
aligned               92 ns/send
aligned, size =    1  92 ns/send
aligned, size =    8  93 ns/send
aligned, size =   16  92 ns/send
aligned, size =   32  92 ns/send
aligned, size =   64  92 ns/send
aligned, size =  128  92 ns/send
aligned, size =  256  92 ns/send
aligned, size =  512  92 ns/send
aligned, size = 1024  92 ns/send
----
stream baseline      986 ns/send
aligned              986 ns/send
no cache             961 ns/send
aligned, no cache    961 ns/send
less contend         359 ns/send
less contend aligned 360 ns/send
----
stream2 baseline     981 ns/send
aligned              986 ns/send
no cache             960 ns/send
aligned, no cache    963 ns/send
less contend         355 ns/send
less contend aligned 354 ns/send
```
```
spsc stream        318 ns/send
spsc shared        859 ns/send
----
mpmc baseline      726 ns/send
aligned            729 ns/send
----
spsc baseline      190 ns/send
bigger cache       738 ns/send
aligned            755 ns/send
unbounded           92 ns/send
no cache           730 ns/send
unbounded, aligned  88 ns/send
no cache, aligned  736 ns/send
----
less contention spsc 122 ns/send
aligned               92 ns/send
aligned, size =    1  92 ns/send
aligned, size =    8  92 ns/send
aligned, size =   16  92 ns/send
aligned, size =   32  92 ns/send
aligned, size =   64  92 ns/send
aligned, size =  128  92 ns/send
aligned, size =  256  92 ns/send
aligned, size =  512  92 ns/send
aligned, size = 1024  92 ns/send
----
stream baseline      1025 ns/send
aligned              1023 ns/send
no cache             1024 ns/send
aligned, no cache    995 ns/send
less contend         401 ns/send
less contend aligned 399 ns/send
----
stream2 baseline     998 ns/send
aligned              996 ns/send
no cache             970 ns/send
aligned, no cache    970 ns/send
less contend         363 ns/send
less contend aligned 362 ns/send
---
spsc stream        1012 ns/send  //XXX this is a rerun of the first benchmark, due to processor throttling it is much slower, since this run is consistent with stream it is more likely to be comparable
```
