/*
CondVar für multiprocessing implementieren (um zu notifyen wenn daten fertig gelesen / geschrieben sind)
*/
#![allow(dead_code)]
use std::cell::UnsafeCell;
use std::io;
use std::io::{Error, ErrorKind, Read, Write};
use std::mem::size_of;
use std::pin::Pin;
use std::str;
use std::time::Instant;

use memmap::{MmapMut, MmapOptions};
use nix::unistd::{fork, ForkResult, Pid};
use sysinfo::{ProcessExt, System, SystemExt};

type Rank = usize;
const SENDER: u8 = 0;
const RECEIVER: u8 = 1;
struct Channel {}

#[derive(Debug)]
struct TransferBuffer<T> {
    mmap: MmapMut,
    pd: std::marker::PhantomData<T>,
}

impl<T> TransferBuffer<T> {
    pub fn new(owner: u8) -> io::Result<Self> {
        let mut mmap_options = MmapOptions::new();
        mmap_options
            .len(size_of::<T>() + 1)
            .map_anon()
            .map(|mmap| TransferBuffer {
                mmap,
                pd: std::marker::PhantomData,
            })
            .map(|mut buf| {
                buf.write_owner(owner);
                buf
            })
    }

    /// Get a pointer to the part of the buffer containing the owner
    fn owner(&self) -> *const u8 {
        &self.mmap[self.size()]
    }

    fn owner_mut(&mut self) -> *mut u8 {
        let i = self.size();
        &mut self.mmap[i]
    }
    /// get the Part of the buffer that's for the actual payload
    fn data_buffer(&self) -> &[u8] {
        &self.mmap[..self.size() - 1]
    }

    fn data_buffer_mut(&mut self) -> &mut [u8] {
        let i = self.size();
        &mut self.mmap[..i - 1]
    }

    /// Returns the size of the data buffer
    fn size(&self) -> usize {
        self.mmap.len() - 1
    }

    /// Set a new owner
    pub fn write_owner(&mut self, owner_id: u8) {
        unsafe { self.owner_mut().write_volatile(owner_id) }
    }

    /// Query the current owner
    pub fn current_owner(&self) -> u8 {
        unsafe { self.owner().read_volatile() }
    }

    /// Block until the current owner is a certain one
    pub fn wait_for_owner(&self, owner_id: u8) -> &Self {
        self.current_owner();
        while self.current_owner() != owner_id {}
        self
    }

    pub fn put(&self, data: T) -> Result<(), ()> {
        let payload_size = size_of::<T>();
        let send_data =
            unsafe { std::slice::from_raw_parts(&data as *const T as *const u8, payload_size) };
        match self.write(send_data) {
            Ok(bytes) if bytes == payload_size => Ok(()),
            _ => Err(()),
        }
    }

    pub fn 
}

impl<T> Write for TransferBuffer<T> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        (&mut self.data_buffer_mut()[..data.len()]).write(data)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.mmap.flush()
    }
}

impl<T> Read for TransferBuffer<T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        (&self.data_buffer()[..]).read(buf)
    }
}

#[derive(Debug)]
pub struct Sender<'a, T> {
    buffer: UnsafeCell<&'a mut TransferBuffer<T>>,
}

impl<'a, T> Sender<'a, T> {
    fn get_buffer_ref(&self) -> Result<&'a TransferBuffer<T>> {
        unsafe { self.buffer.get().as_ref() }
            .map(|x| &**x)
            .ok_or(Error::new(
                ErrorKind::Other,
                "Failed to get reference to buffer",
            ))
    }

    fn get_buffer_mut(&mut self) -> Result<&'a mut TransferBuffer> {
        unsafe { self.buffer.get().as_mut() }
            .map(|x| &mut **x)
            .ok_or(Error::new(
                ErrorKind::Other,
                "Failed to get mutable reference to buffer",
            ))
    }
}

impl Write for Sender<'_> {
    fn write(&mut self, data: &[u8]) -> Result<usize> {
        self.get_buffer_ref()?.wait_for_owner(SENDER);
        let buf = self.get_buffer_mut()?;
        let w = (&mut buf.buffer_mut()[..data.len()]).write(data)?;
        buf.write_owner(RECEIVER);
        Ok(w)
    }

    fn flush(&mut self) -> Result<()> {
        let buf = self.get_buffer_mut()?;
        (&mut buf.buffer_mut()[..]).flush()
    }
}

#[derive(Debug)]
pub struct Receiver {
    buffer: TransferBuffer,
}

impl Receiver {
    pub fn new(buffer_size: usize) -> Result<Self> {
        let buffer = TransferBuffer::new(buffer_size, SENDER)?;
        Ok(Receiver { buffer })
    }

    pub fn new_sender(&mut self) -> Sender {
        let pointer = &mut self.buffer;
        Sender {
            buffer: UnsafeCell::new(pointer),
        }
    }
}

impl Read for Receiver {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.buffer.wait_for_owner(RECEIVER);
        let r = (&self.buffer.buffer()[..buf.len()]).read(buf)?;
        self.buffer.write_owner(SENDER);
        Ok(r)
    }
}

/*
pub fn channel<'a>(buffer_size: usize) -> Result<(Receiver, Sender<'a>)> {
    let mut rx = Receiver::new(buffer_size)?;
    let tx = rx.new_sender();
    Ok((rx, tx))
}
*/

pub fn main() {
    let mut receiver = Receiver::new(6).unwrap();
    let mut sender = receiver.new_sender();
    sender.write(&[1, 2, 3, 4, 5]).unwrap();
    let mut outbuf = [0; 5];
    receiver.read(&mut outbuf).unwrap();
    dbg!(outbuf);
}

//std::slice::from_raw_parts für generic channel
