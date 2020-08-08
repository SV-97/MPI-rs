# MPI-rs

Basic Message Passing and interprocess communication stuff written in Rust.

* `mpi1` contains some basic benchmarks and implementations based on the book "Inside the message passing interface"
* In `ipc` I started building a Rust wrapper around System-V Semaphores because I thought I'd need them.
* `mpi2` contains an unbuffered, unidirectional, typed channel for blocking multiprocess communication. This implementation outperforms the servo ipc-channels in my benchmarks by at least one order of magnitude. It's very well possible that there's some terrible bugs in my implementation that I just haven't noticed until now, because there's quite a bit of unsafe code.
