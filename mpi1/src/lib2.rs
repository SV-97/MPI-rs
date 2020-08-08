#![allow(dead_code)]
use std::cell::UnsafeCell;
use std::io::{Error, ErrorKind, Read, Result, Write};

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

    pub fn wait_for_owner(&self, owner_id: u8) -> &Self {
        self.current_owner();
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
        (&self.buffer()[..]).read(buf)
    }
}

impl TransferBuffer {}

#[derive(Debug)]
pub struct Sender<'a> {
    buffer: UnsafeCell<&'a mut TransferBuffer>,
}

impl<'a> Sender<'a> {
    fn get_buffer_ref(&self) -> Result<&'a TransferBuffer> {
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
