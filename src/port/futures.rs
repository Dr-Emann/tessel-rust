use super::raw;
use super::Port;
use super::ReadRequest;

use std::io;

use futures;
use futures::{Future, Poll, Async, Oneshot};
use futures::stream::Stream;
use tokio_uds::UnixStream;
use tokio_core::io as tokio_io;
use tokio_core::channel::{channel, Receiver};

pub struct PinRead<'a> {
    port: &'a Port,
    state: PinReadState<'a>,
}

impl<'a> PinRead<'a> {
    pub fn new(port: &'a Port, pin: u8) -> PinRead<'a> {
        PinRead {
            port: port,
            state: PinReadState::new(&*port.stream, [raw::cmd::GPIO_IN, pin]),
        }
    }
}

enum PinReadState<'a> {
    Writing(tokio_io::WriteAll<&'a UnixStream, [u8; 2]>),
    Reading(Oneshot<bool>),
}

impl<'a> PinReadState<'a> {
    fn new(stream: &'a UnixStream, data: [u8; 2]) -> PinReadState<'a> {
        PinReadState::Writing(tokio_io::write_all(stream, data))
    }
}

impl<'a> Future for PinRead<'a> {
    type Item = bool;
    type Error = io::Error;
    fn poll(&mut self) -> Poll<bool, io::Error> {
        match self.state {
            PinReadState::Writing(ref mut future) => {
                match try!(future.poll()) {
                    Async::NotReady => return Ok(Async::NotReady),
                    _ => {}
                }
            },
            PinReadState::Reading(ref mut future) => {
                return future.poll().map_err(|_| io::Error::new(io::ErrorKind::Other, "Oneshot was closed"))
            },
        }
        // Only run after Writing completes
        let (tx, mut rx) = futures::oneshot();
        // limit length of refcell borrow
        {
            let mut requests = self.port.read_requests.borrow_mut();
            requests.push_front(ReadRequest::Pin(tx));
        }
        let result = rx.poll().map_err(|_| io::Error::new(io::ErrorKind::Other, "Oneshot was closed"));
        self.state = PinReadState::Reading(rx);
        result
    }
}


pub struct PinChanges<'a> {
    port: &'a Port,
    pin: u8,
    state: PinChangesState<'a>,
}

impl<'a> PinChanges<'a> {
    pub fn new(port: &'a Port, pin: u8) -> PinChanges<'a> {
        PinChanges {
            port: port,
            pin: pin,
            state: PinChangesState::Writing(
                tokio_io::write_all(&*port.stream,
                                     [raw::cmd::GPIO_INT, pin | raw::interrupt_mode::change << 4])
            ),
        }
    }
}

enum PinChangesState<'a> {
    Writing(tokio_io::WriteAll<&'a UnixStream, [u8; 2]>),
    Waiting(Receiver<bool>),
}

impl<'a> Stream for PinChanges<'a> {
    type Item = bool;
    type Error = io::Error;
    fn poll(&mut self) -> Poll<Option<bool>, io::Error> {
        match self.state {
            PinChangesState::Writing(ref mut future) => {
                match try!(future.poll()) {
                    Async::NotReady => return Ok(Async::NotReady),
                    _ => {}
                }
            },
            PinChangesState::Waiting(ref mut receiver) => {
                return receiver.poll()
            },
        }
        // Only run after Writing completes
        let (tx, mut rx) = try!(channel(&self.port.handle));
        // limit length of refcell borrow
        {
            let mut callbacks = self.port.callbacks.borrow_mut();
            let callbacks = &mut callbacks[self.pin as usize];
            callbacks.change = Some(tx);
        }
        let result = rx.poll();
        self.state = PinChangesState::Waiting(rx);
        result
    }
}

impl<'a> Drop for PinChanges<'a> {
    fn drop(&mut self) {
        let mut callbacks = self.port.callbacks.borrow_mut();
        let callbacks = &mut callbacks[self.pin as usize];
        callbacks.change = None;
    }
}

pub struct I2cRead<'a> {
    port: &'a Port,
    state: I2cReadState<'a>,
    len: u8,
}

impl<'a> I2cRead<'a> {
    pub fn new(port: &'a Port, address: u8, len: u8) -> I2cRead<'a> {
        let encoded_addr = (address << 1) | 0x1;
        I2cRead {
            port: port,
            len: len,
            state: I2cReadState::Writing(
                tokio_io::write_all(&*port.stream,
                                    [raw::cmd::START,
                                    encoded_addr,
                                    raw::cmd::RX,
                                    len,
                                    raw::cmd::STOP])
            )
        }
    }
}

enum I2cReadState<'a> {
    Writing(tokio_io::WriteAll<&'a UnixStream, [u8; 5]>),
    Waiting(Oneshot<Box<[u8]>>),
}

impl<'a> Future for I2cRead<'a> {
    type Item = Box<[u8]>;
    type Error = io::Error;
    fn poll(&mut self) -> Poll<Box<[u8]>, io::Error> {
        match self.state {
            I2cReadState::Writing(ref mut future) => {
                match try!(future.poll()) {
                    Async::NotReady => return Ok(Async::NotReady),
                    _ => {}
                }
            },
            I2cReadState::Waiting(ref mut oneshot) => {
                return oneshot.poll().map_err(|_| io::Error::new(io::ErrorKind::Other, "Oneshot was closed"))
            },
        }
        // Only run after Writing completes
        let (tx, mut rx) = futures::oneshot();
        // limit length of refcell borrow
        {
            let mut requests = self.port.read_requests.borrow_mut();
            requests.push_front(ReadRequest::Data(self.len as usize, tx));
        }
        let result = rx.poll().map_err(|_| io::Error::new(io::ErrorKind::Other, "Oneshot was closed"));
        self.state = I2cReadState::Waiting(rx);
        result
    }
}
