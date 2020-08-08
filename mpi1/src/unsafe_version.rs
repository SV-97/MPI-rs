use std::io::{Read, Write};
use std::str;
use std::time::Instant;

use memmap::{MmapMut, MmapOptions};
use nix::unistd::{fork, ForkResult};
use sysinfo::{ProcessExt, System, SystemExt};

const IMAX: usize = 100000;
const LMAX: usize = 1024 * 256;

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
                    (&transfer_buffer[..message_length]).read(&mut buf);
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
                    10.0f64.powf(-9.0) * message_length as f64 / t2.as_nanos() as f64
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
