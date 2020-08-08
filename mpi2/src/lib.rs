#![allow(dead_code)]
use std::cell::UnsafeCell;
use std::io;
use std::io::{Error, ErrorKind, Read, Write};
use std::marker::PhantomData;
use std::mem::size_of;
use std::time::{Duration, Instant};

use memmap::{MmapMut, MmapOptions};
use nix::unistd::{fork, ForkResult, Pid};
use sysinfo::{Process, ProcessExt, Signal, System, SystemExt};

type Rank = usize;
const SENDER: u8 = 0;
const RECEIVER: u8 = 1;

fn kill_process(process: &Process) {
    if !process.kill(Signal::Abort) {
        process.kill(Signal::Kill);
    }
}

fn wait_for_process<F: FnOnce(&Process)>(pid: Pid, timeout: Option<(Duration, F)>) {
    let mut sys = System::new();
    sys.refresh_all();
    let t1 = Instant::now();
    if let Some(p) = sys.get_process(i32::from(pid)) {
        match timeout {
            Some((timeout, action)) => {
                while p.status().to_string() != "Zombie" {
                    // yup, this is shit code.
                    if (Instant::now() - t1) >= timeout {
                        action(&p);
                        break;
                    }
                }
            }
            None => while p.status().to_string() != "Zombie" {},
        }
    }
}

#[derive(Debug)]
struct TransferBuffer {
    mmap: MmapMut,
}

impl TransferBuffer {
    pub fn new(size: usize, owner: u8) -> io::Result<Self> {
        let mut mmap_options = MmapOptions::new();
        mmap_options
            .len(size + 2)
            .map_anon()
            .map(|mmap| TransferBuffer { mmap })
            .map(|mut buf| {
                buf.write_owner(owner);
                buf
            })
    }

    fn owner(&self) -> *const u8 {
        &self.mmap[self.size()]
    }

    fn buffer(&self) -> &[u8] {
        &self.mmap[..self.size() - 1]
    }

    fn owner_mut(&mut self) -> *mut u8 {
        let i = self.size();
        &mut self.mmap[i]
    }

    fn buffer_mut(&mut self) -> &mut [u8] {
        let i = self.size();
        &mut self.mmap[..i - 1]
    }

    /// Returns the size of the data buffer
    fn size(&self) -> usize {
        self.mmap.len() - 1
    }

    pub fn write_owner(&mut self, owner_id: u8) {
        unsafe { self.owner_mut().write_volatile(owner_id) }
    }

    pub fn current_owner(&self) -> u8 {
        unsafe { self.owner().read_volatile() }
    }

    pub fn wait_for_owner(&self, owner_id: u8) -> &Self {
        self.current_owner();
        while self.current_owner() != owner_id {}
        self
    }
}

impl Write for TransferBuffer {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        (&mut self.buffer_mut()[..data.len()]).write(data)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.mmap.flush()
    }
}

impl Read for TransferBuffer {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&self.buffer()[..]).read(buf)
    }
}

#[derive(Debug)]
pub struct Sender<'a, T> {
    buffer: UnsafeCell<&'a mut TransferBuffer>,
    phantom_data: PhantomData<T>,
}

impl<'a, T> Sender<'a, T> {
    fn get_buffer_ref(&self) -> io::Result<&'a TransferBuffer> {
        unsafe { self.buffer.get().as_ref() }
            .map(|x| &**x)
            .ok_or_else(|| Error::new(ErrorKind::Other, "Failed to get reference to buffer"))
    }

    fn get_buffer_mut(&mut self) -> io::Result<&'a mut TransferBuffer> {
        unsafe { self.buffer.get().as_mut() }
            .map(|x| &mut **x)
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::Other,
                    "Failed to get mutable reference to buffer",
                )
            })
    }

    /// Put data into the channel
    pub fn send(&mut self, data: &T) -> Result<(), ()> {
        let payload_size = size_of::<T>();
        let send_data =
            unsafe { std::slice::from_raw_parts(data as *const T as *const u8, payload_size) };
        match self.write(send_data) {
            Ok(bytes) if bytes == payload_size => Ok(()),
            _ => Err(()),
        }
    }
}

impl<T> Write for Sender<'_, T> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.get_buffer_ref()?.wait_for_owner(SENDER);
        let buf = self.get_buffer_mut()?;
        let w = (&mut buf.buffer_mut()[..data.len()]).write(data)?;
        buf.write_owner(RECEIVER);
        Ok(w)
    }

    fn flush(&mut self) -> io::Result<()> {
        let buf = self.get_buffer_mut()?;
        (&mut buf.buffer_mut()[..]).flush()
    }
}

#[derive(Debug)]
pub struct Receiver<T> {
    buffer: TransferBuffer,
    phantom_data: PhantomData<T>,
}

impl<T: Copy> Receiver<T> {
    pub fn new() -> io::Result<Self> {
        let buffer_size = size_of::<T>();
        let buffer = TransferBuffer::new(buffer_size, SENDER)?;
        Ok(Receiver {
            buffer,
            phantom_data: PhantomData,
        })
    }

    pub fn new_sender(&mut self) -> Sender<T> {
        let pointer = &mut self.buffer;
        Sender {
            buffer: UnsafeCell::new(pointer),
            phantom_data: PhantomData,
        }
    }
}

impl<T: Copy + Sized> Receiver<T> {
    pub fn recv(&mut self) -> io::Result<T> {
        /* The buffer should really be a stack allocated array for best performance.
        Sadly this is currently not possible since the constant can't depend on the
        type parameter. This is likely going to be fixed in a future version of Rust.
        See also: https://github.com/rust-lang/rust/pull/68388
        Correct code should be:
        let mut buf: [u8; size_of::<T>()] = [0; size_of::<T>()];
        self.read(&mut buf)?;
        let (_head, body, _tail) = unsafe { buf.align_to::<T>() };
        Ok(body[0])

        The current workaround is ghetto as hell for performance reasons.
        We'll just use a buffer that's large enough for "most" types; using a Vec would be
        another option but the incurred cost of the heap allocation brings down the performance
        by a factor of about 5 (on my machine).
        */
        const BUFFER_SIZE: usize = 1024 * 1024;
        assert!(size_of::<T>() <= BUFFER_SIZE);
        let mut buf: [u8; BUFFER_SIZE] = [0; BUFFER_SIZE];
        self.read(&mut buf)?;
        let (_head, body, _tail) = unsafe { buf.align_to::<T>() };
        Ok(body[0])
    }
}

impl<T> Read for Receiver<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.buffer.wait_for_owner(RECEIVER);
        let r = (&self.buffer.buffer()[..]).read(buf)?;
        self.buffer.write_owner(SENDER);
        Ok(r)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[derive(Debug, Copy, Clone, PartialEq, Default)]
    struct Test {
        a: usize,
        b: i32,
        c: f64,
    }
    impl Test {
        pub fn new(a: usize, b: i32, c: f64) -> Test {
            Test { a, b, c }
        }
    }

    #[test]
    pub fn simple_transfer() {
        let mut receiver1 = Receiver::<usize>::new().unwrap();
        let mut sender1 = receiver1.new_sender();

        let mut receiver2 = Receiver::<[i32; 20]>::new().unwrap();
        let mut sender2 = receiver2.new_sender();
        let data2 = [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, -10, -9, -8, -7, -6, -5, -4, -3, -2, -1,
        ];

        let mut receiver3 = Receiver::<Test>::new().unwrap();
        let mut sender3 = receiver3.new_sender();
        let data3 = Test::new(420, -69, 3.14);

        match fork() {
            Ok(ForkResult::Parent { child, .. }) => {
                sender1.send(&123).unwrap();
                sender1.send(&456).unwrap();
                sender2.send(&data2).unwrap();
                assert_eq!(receiver3.recv().unwrap(), data3);
                wait_for_process::<fn(&Process)>(child, None);
            }
            Ok(ForkResult::Child) => {
                assert_eq!(receiver1.recv().unwrap(), 123);
                assert_eq!(receiver1.recv().unwrap(), 456);
                assert_eq!(receiver2.recv().unwrap(), data2);
                sender3.send(&data3).unwrap();
            }
            Err(e) => panic!("fork failed: {}", e),
        }
    }
}

pub fn bench_data_rate() {
    const BUFFER_SIZE: usize = 1024 * 1024; // set back to 32 if you want to compare to servo
    const IMAX: usize = 100_000;
    const LENGTHS: usize = 3;

    let mut receiver = Receiver::<[u8; BUFFER_SIZE]>::new().unwrap();
    let mut sender = receiver.new_sender();
    match fork() {
        Ok(ForkResult::Parent { child, .. }) => {
            let mut times = Vec::new();
            let pid = std::process::id();
            println!("Receiver: {}, Sender: {}", pid, child);

            for _ in 0..LENGTHS {
                let t1 = Instant::now();
                for _ in 0..IMAX {
                    let _dat = receiver.recv().unwrap();
                }
                let t2 = Instant::now() - t1;
                times.push((BUFFER_SIZE, t2));
            }

            for (message_length, t2) in times {
                println!(
                    "Rx, pid: {:?}, length: {:-6}, time: {:?}, latency: {:?}, bandwith: {:e}byte/s",
                    pid,
                    message_length,
                    t2,
                    t2.checked_div(IMAX as u32).unwrap(),
                    10.0f64.powf(9.0) * (message_length * IMAX) as f64 / t2.as_nanos() as f64
                );
            }
            wait_for_process(child, Some((Duration::from_secs(10), &kill_process)));
            println!("Parent shutting down");
        }
        Ok(ForkResult::Child) => {
            // sender
            let mut times = Vec::new();
            let pid = std::process::id();
            let buf = [0; BUFFER_SIZE];

            for _ in 0..LENGTHS {
                let t1 = Instant::now();
                for _ in 0..IMAX {
                    sender.send(&buf).unwrap();
                }
                let t2 = Instant::now() - t1;
                times.push((BUFFER_SIZE, t2));
            }

            for (message_length, t2) in times {
                println!(
                    "Tx, pid: {:?}, length: {:-6}, time: {:?}, latency: {:?}, bandwith: {:e}byte/s",
                    pid,
                    message_length,
                    t2,
                    t2.checked_div(IMAX as u32).unwrap(),
                    10.0f64.powf(9.0) * (message_length * IMAX) as f64 / t2.as_nanos() as f64
                );
            }
            println!("Child shutting down");
        }
        Err(_) => panic!("Fork failed"),
    }
}

pub fn bench_data_rate_servo() {
    use ipc_channel::ipc;

    const BUFFER_SIZE: usize = 32;
    const IMAX: usize = 100_000;
    const LENGTHS: usize = 3;

    let (tx, rx) = ipc::channel().unwrap();
    match fork() {
        Ok(ForkResult::Parent { child, .. }) => {
            let mut times = Vec::new();
            let pid = std::process::id();
            println!("Receiver: {}, Sender: {}", pid, child);

            for _ in 0..LENGTHS {
                let t1 = Instant::now();
                for _ in 0..IMAX {
                    let _dat = rx.recv().unwrap();
                }
                let t2 = Instant::now() - t1;
                times.push((BUFFER_SIZE, t2));
            }

            for (message_length, t2) in times {
                println!(
                    "Rx, pid: {:?}, length: {:-6}, time: {:?}, latency: {:?}, bandwith: {:e}byte/s",
                    pid,
                    message_length,
                    t2,
                    t2.checked_div(IMAX as u32).unwrap(),
                    10.0f64.powf(9.0) * (message_length * IMAX) as f64 / t2.as_nanos() as f64
                );
            }
            wait_for_process(child, Some((Duration::from_secs(10), &kill_process)));
            println!("Parent shutting down");
        }
        Ok(ForkResult::Child) => {
            // sender
            let mut times = Vec::new();
            let pid = std::process::id();
            let buf = [0u8; BUFFER_SIZE];

            for _ in 0..LENGTHS {
                let t1 = Instant::now();
                for _ in 0..IMAX {
                    tx.send(buf).unwrap();
                }
                let t2 = Instant::now() - t1;
                times.push((BUFFER_SIZE, t2));
            }

            for (message_length, t2) in times {
                println!(
                    "Tx, pid: {:?}, length: {:-6}, time: {:?}, latency: {:?}, bandwith: {:e}byte/s",
                    pid,
                    message_length,
                    t2,
                    t2.checked_div(IMAX as u32).unwrap(),
                    10.0f64.powf(9.0) * (message_length * IMAX) as f64 / t2.as_nanos() as f64
                );
            }
            println!("Child shutting down");
        }
        Err(_) => panic!("Fork failed"),
    }
}
