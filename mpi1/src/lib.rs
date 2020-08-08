#![allow(dead_code)]
use std::io::{Read, Result, Write};
use std::str;
use std::time::Instant;

use memmap::{MmapMut, MmapOptions};
use nix::unistd::{fork, ForkResult, Pid};
use sysinfo::{ProcessExt, System, SystemExt};

const IMAX: usize = 100000;
const LMAX: usize = 1024 * 256;

fn wait_for_process(pid: Pid) {
    let mut sys = System::new();
    sys.refresh_all();
    if let Some(p) = sys.get_process(i32::from(pid)) {
        while p.status().to_string() != "Zombie" {}
    }
}

struct TransferBuffer {
    mmap: MmapMut,
}

impl TransferBuffer {
    pub fn new(size: usize, owner: u8) -> Result<Self> {
        let mut mmap_options = MmapOptions::new();
        mmap_options
            .len(size + 1)
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

    pub fn wait_for_owner(&mut self, owner_id: u8) -> &mut Self {
        while self.current_owner() != owner_id {}
        self
    }
}

impl Write for TransferBuffer {
    fn write(&mut self, data: &[u8]) -> Result<usize> {
        (&mut self.buffer_mut()[..data.len()]).write(data)
    }
    fn flush(&mut self) -> Result<()> {
        self.mmap.flush()
    }
}

impl Read for TransferBuffer {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        (&self.buffer()[..buf.len()]).read(buf)
    }
}

pub fn nicer_naive_shared_memory_benchmark() {
    const TX: u8 = 0;
    const RX: u8 = 1;
    let mut transfer_buffer = TransferBuffer::new(LMAX + 1, 0).expect("mmap failed");
    let message_lengths = (0..).map(|i| (2 as usize).pow(i)).take_while(|i| i < &LMAX);
    match fork() {
        Ok(ForkResult::Parent { child, .. }) => {
            let mut times = Vec::with_capacity(LMAX);
            let pid = std::process::id();
            println!("Receiver: {}, Sender: {}", pid, child);
            // receiver
            for message_length in message_lengths {
                let mut buf = [0; LMAX];
                let t1 = Instant::now();
                for _ in 0..IMAX {
                    transfer_buffer.wait_for_owner(RX).read(&mut buf).unwrap();
                }
                let t2 = Instant::now() - t1;
                times.push((message_length, t2));
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
            wait_for_process(child);
            println!("Parent shutting down");
        }
        Ok(ForkResult::Child) => {
            // sender
            let mut times = Vec::with_capacity(LMAX);
            let pid = std::process::id();
            let buf = [0; LMAX];
            for message_length in message_lengths {
                let t1 = Instant::now();
                for _ in 0..IMAX {
                    transfer_buffer
                        .wait_for_owner(TX)
                        .write_all(&buf[..message_length])
                        .unwrap();
                }
                let t2 = Instant::now() - t1;
                times.push((message_length, t2));
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

pub fn naive_shared_memory_benchmark() {
    let mut mmap_options = MmapOptions::new();
    let mut mmap: MmapMut = mmap_options
        .len(LMAX + 1)
        .map_anon()
        .expect("Memory map failed");
    let shm: *mut u8 = &mut mmap[LMAX];
    let transfer_buffer = &mut mmap[..LMAX - 1];
    let message_lengths = (0..).map(|i| (2 as usize).pow(i)).take_while(|i| i < &LMAX);
    match fork() {
        Ok(ForkResult::Parent { child, .. }) => {
            let mut times = Vec::with_capacity(LMAX);
            let pid = std::process::id();
            println!("Receiver: {}, Sender: {}", pid, child);
            // receiver
            for message_length in message_lengths {
                let mut buf = [0; LMAX];
                let t1 = Instant::now();
                for _ in 0..IMAX {
                    while unsafe { shm.read_volatile() } != 1 {} // Rx waiting
                    (&transfer_buffer[..message_length]).read(&mut buf).unwrap();
                    unsafe {
                        shm.write_volatile(0);
                    }
                }
                let t2 = Instant::now() - t1;
                times.push((message_length, t2));
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
            let mut sys = System::new();
            sys.refresh_all();
            if let Some(p) = sys.get_process(i32::from(child)) {
                while p.status().to_string() != "Zombie" {}
            }
            println!("Parent shutting down");
        }
        Ok(ForkResult::Child) => {
            // sender
            let mut times = Vec::with_capacity(LMAX);
            let pid = std::process::id();
            let buf = [0; LMAX];
            for message_length in message_lengths {
                let t1 = Instant::now();
                for _ in 0..IMAX {
                    while unsafe { shm.read_volatile() } != 0 {} // Tx waiting
                    (&mut transfer_buffer[..message_length])
                        .write_all(&buf[..message_length])
                        .unwrap();
                    unsafe {
                        shm.write_volatile(1);
                    }
                }
                let t2 = Instant::now() - t1;
                times.push((message_length, t2));
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

pub fn simple_message_passing() {
    let mut mmap_options = MmapOptions::new();
    let mut mmap: MmapMut = mmap_options
        .len(LMAX + 1)
        .map_anon()
        .expect("Memory map failed");

    match fork() {
        Ok(ForkResult::Parent { .. }) => {
            // receiver
            let mmap = mmap.make_read_only().unwrap();
            let mut buf = [0; LMAX];
            while mmap[LMAX] != 1 {}
            (&mmap[..]).read(&mut buf).unwrap();
            dbg!(str::from_utf8(&buf[..])
                .unwrap()
                .to_owned()
                .trim_matches(char::from(0)));
        }
        Ok(ForkResult::Child) => {
            // sender
            (&mut mmap[..]).write_all(b"Hi").unwrap();
            mmap[LMAX] = 1;
            mmap.flush().unwrap();
        }
        Err(_) => panic!("Fork failed"),
    }
}

pub fn memory_benchmark() {
    const LEN: usize = 3200000000;
    const I_MAX: usize = 100_000;
    let buf1 = vec![0; LEN];
    let mut buf2 = vec![1; LEN];

    let lengths = (0..).map(|i| (2 as usize).pow(i)).take_while(|i| i <= &LEN);
    for l in lengths {
        let t1 = Instant::now();
        for _ in 0..I_MAX {
            criterion::black_box((&mut buf2[..l]).write(&buf1[..l]).unwrap());
        }
        let t2 = Instant::now() - t1;
        println!(
            "bytes: {:-16}, time: {:?}, bandwidth: {:e} bytes/s",
            l,
            t2,
            10.0f64.powf(9.0) * (l * I_MAX) as f64 / t2.as_nanos() as f64
        );
    }
}
