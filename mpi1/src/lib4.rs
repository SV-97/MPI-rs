#![allow(dead_code)]
use std::cell::UnsafeCell;
use std::io;
use std::io::{Error, ErrorKind, Read, Write};
use std::marker::PhantomData;
use std::mem::size_of;

use memmap::{MmapMut, MmapOptions};

type Rank = usize;
const SENDER: u8 = 0;
const RECEIVER: u8 = 1;
struct Channel {}

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

impl TransferBuffer {}

#[derive(Debug)]
pub struct Sender<'a, T> {
    buffer: UnsafeCell<&'a mut TransferBuffer>,
    phantom_data: PhantomData<T>,
}

impl<'a, T> Sender<'a, T> {
    fn get_buffer_ref(&self) -> io::Result<&'a TransferBuffer> {
        unsafe { self.buffer.get().as_ref() }
            .map(|x| &**x)
            .ok_or(Error::new(
                ErrorKind::Other,
                "Failed to get reference to buffer",
            ))
    }

    fn get_buffer_mut(&mut self) -> io::Result<&'a mut TransferBuffer> {
        unsafe { self.buffer.get().as_mut() }
            .map(|x| &mut **x)
            .ok_or(Error::new(
                ErrorKind::Other,
                "Failed to get mutable reference to buffer",
            ))
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

    pub fn get(&mut self) -> io::Result<T> {
        // let mut buf: [u8; size_of::<T>()] = [0; size_of::<T>()];
        let mut buf: Vec<u8> = vec![0; size_of::<T>()];
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

mod tests {
    use super::*;

    use nix::unistd::{fork, ForkResult};
    #[derive(Debug, Copy, Clone, PartialEq)]
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
    fn simple_transfer() {
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
            Ok(ForkResult::Parent { .. }) => {
                sender1.send(&123).unwrap();
                sender1.send(&456).unwrap();
                sender2.send(&data2).unwrap();
                assert_eq!(receiver3.get().unwrap(), data3);
            }
            Ok(ForkResult::Child) => {
                assert_eq!(receiver1.get().unwrap(), 123);
                assert_eq!(receiver1.get().unwrap(), 456);
                assert_eq!(receiver2.get().unwrap(), data2);
                sender3.send(&data3).unwrap();
            }
            Err(e) => panic!("fork failed: {}", e),
        }
    }
}

pub fn main() {}
