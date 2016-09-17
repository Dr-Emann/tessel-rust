extern crate futures;
#[macro_use]
extern crate tokio_core;
extern crate tokio_uds;

use std::io;
use std::io::prelude::*;
use std::path::Path;
use std::cell::RefCell;
use std::rc::Rc;
use std::collections::VecDeque;
use std::cmp;
use std::mem;

use futures::{Complete, Oneshot, Future, Poll, Async};
use futures::stream::Stream;
use tokio_uds::UnixStream;
use tokio_core::io as tokio_io;
use tokio_core::reactor::{Core, Handle};
use tokio_core::channel::{channel, Sender, Receiver};

const ANALOG_RESOLUTION: u16 = 0x1000;

#[allow(dead_code)]
mod raw_cmd {
    pub const NOP: u8 = 0x00;
    pub const FLUSH: u8 = 0x01;
    pub const ECHO: u8 = 0x02;
    pub const GPIO_IN: u8 = 0x03;
    pub const GPIO_HIGH: u8 = 0x04;
    pub const GPIO_LOW: u8 = 0x05;
    pub const GPIO_CFG: u8 = 0x06;
    pub const GPIO_WAIT: u8 = 0x07;
    pub const GPIO_INT: u8 = 0x08;
    pub const ENABLE_SPI: u8 = 0x0A;
    pub const DISABLE_SPI: u8 = 0x0B;
    pub const ENABLE_I2C: u8 = 0x0C;
    pub const DISABLE_I2C: u8 = 0x0D;
    pub const ENABLE_UART: u8 = 0x0E;
    pub const DISABLE_UART: u8 = 0x0F;
    pub const TX: u8 = 0x10;
    pub const RX: u8 = 0x11;
    pub const TXRX: u8 = 0x12;
    pub const START: u8 = 0x13;
    pub const STOP: u8 = 0x14;
    pub const GPIO_TOGGLE: u8 = 0x15;
    pub const GPIO_INPUT: u8 = 0x16;
    pub const GPIO_RAW_READ: u8 = 0x17;
    pub const ANALOG_READ: u8 = 0x18;
    pub const ANALOG_WRITE: u8 = 0x19;
    pub const GPIO_PULL: u8 = 0x1A;
    pub const PWM_DUTY_CYCLE: u8 = 0x1B;
    pub const PWM_PERIOD: u8 = 0x1C;
}

/// Starting byte of reply packets. Because this is extensible, we use
/// a list of constants instead of an enum.
#[allow(dead_code)]
mod raw_reply {
    pub const ACK: u8 = 0x80;
    pub const NACK: u8 = 0x81;
    pub const HIGH: u8 = 0x82;
    pub const LOW: u8 = 0x83;
    pub const DATA: u8 = 0x84;

    pub const MIN_ASYNC: u8 = 0xA0;
    /// c0 to c8 is all async pin assignments.
    pub const ASYNC_PIN_CHANGE_N: u8 = 0xC0;
    pub const ASYNC_UART_RX: u8 = 0xD0;
}

const PORT_A_PATH: &'static str = "/var/run/tessel/port_a";
const PORT_B_PATH: &'static str = "/var/run/tessel/port_b";

pub struct Tessel {
    port_a: Port,
    port_b: Port,
}

impl Tessel {
    fn new(handle: &Handle) -> io::Result<Tessel> {
        let port_a = try!(Port::connect(PORT_A_PATH, handle));
        let port_b = try!(Port::connect(PORT_B_PATH, handle));
        Ok(Tessel {
            port_a: port_a,
            port_b: port_b,
        })
    }
}

#[derive(Default)]
struct PinCallbacks {
    rise: Option<Sender<bool>>,
    fall: Option<Sender<bool>>,
    change: Option<Sender<bool>>,
    high: Option<Complete<bool>>,
    low: Option<Complete<bool>>,
}

pub struct Port {
    stream: Rc<UnixStream>,
    callbacks: Rc<RefCell<[PinCallbacks; 8]>>,
    read_requests: Rc<RefCell<VecDeque<ReadRequest>>>,
}

enum ReadRequest {
    Data(usize, Complete<Box<[u8]>>),
    Pin(Complete<bool>),
}

impl Port {
    pub fn connect<P: AsRef<Path> + ?Sized>(path: &P, handle: &Handle) -> io::Result<Port> {
        Port::_connect(path.as_ref(), handle)
    }

    fn _connect(path: &Path, handle: &Handle) -> io::Result<Port> {
        let stream = Rc::new(try!(UnixStream::connect(path, handle)));
        let callbacks = Rc::new(RefCell::new(Default::default()));
        let requests = Rc::new(RefCell::new(VecDeque::new()));
        let reads = Reads::new(stream.clone(), callbacks.clone(), requests.clone());
        handle.spawn(reads.map_err(|e| panic!("{}", e)));
        Ok(Port{
            stream: stream,
            callbacks: callbacks.clone(),
            read_requests: requests,
        })
    }

    fn gpio_digital_write(&self, pin: u8, value: bool) -> tokio_io::WriteAll<&UnixStream, [u8; 2]> {
        let cmd = if value { raw_cmd::GPIO_HIGH } else { raw_cmd::GPIO_LOW };
        tokio_io::write_all(&*self.stream, [cmd, pin])
    }

    fn gpio_digital_read(&self, pin: u8, value: bool) -> TxRx {
        TxRx {
            port: self,
            state: TxRxState::Writing(tokio_io::write_all(&*self.stream, [raw_cmd::GPIO_IN, pin])),
        }
    }
}

struct TxRx<'a> {
    port: &'a Port,
    state: TxRxState<'a>,
}

enum TxRxState<'a> {
    Writing(tokio_io::WriteAll<&'a UnixStream, [u8; 2]>),
    Reading(Oneshot<bool>)
}

impl<'a> Future for TxRx<'a> {
    type Item = bool;
    type Error = io::Error;
    fn poll(&mut self) -> Poll<bool, io::Error> {
        match self.state {
            TxRxState::Writing(ref mut future) => {
                match try!(future.poll()) {
                    Async::NotReady => return Ok(Async::NotReady),
                    _ => {}
                }
            },
            TxRxState::Reading(ref mut future) => {
                return future.poll().map_err(|_| io::Error::new(io::ErrorKind::Other, "Oneshot was closed"))
            },
        }
        // Only run after Writing completes
        let (tx, rx) = futures::oneshot();
        {
            let mut requests = self.port.read_requests.borrow_mut();
            requests.push_front(ReadRequest::Pin(tx));
        }
        self.state = TxRxState::Reading(rx);
        Ok(Async::NotReady)
    }
}

struct Reads {
    stream: Rc<UnixStream>,
    callbacks: Rc<RefCell<[PinCallbacks; 8]>>,
    requests: Rc<RefCell<VecDeque<ReadRequest>>>,
    state: ReadState,
    buf: [u8; 1024],
}

impl Reads {
    fn new(stream: Rc<UnixStream>, callbacks: Rc<RefCell<[PinCallbacks; 8]>>,
           requests: Rc<RefCell<VecDeque<ReadRequest>>>) -> Reads {
        Reads {
            stream: stream,
            callbacks: callbacks,
            requests: requests,
            state: ReadState::Waiting,
            buf: [0; 1024],
        }
    }

    fn handle_pin_interrupt(&self, pin: u8, value: bool) {
        let mut callbacks = self.callbacks.borrow_mut();
        let callbacks = &mut callbacks[pin as usize];
        if let Some(ref cb) = callbacks.change {
            let _ = cb.send(value);
        }
        if value {
            if let Some(ref cb) = callbacks.rise {
                let _ = cb.send(value);
            }
            if let Some(cb) = callbacks.high.take() {
                let _ = cb.complete(value);
            }
        } else {
            if let Some(ref cb) = callbacks.fall {
                let _ = cb.send(value);
            }
            if let Some(cb) = callbacks.low.take() {
                let _ = cb.complete(value);
            }
        }
    }
}

impl Future for Reads {
    type Item = ();
    type Error = io::Error;
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            let mut state = mem::replace(&mut self.state, ReadState::Waiting);
            let count = try_nb!((&*self.stream).read(&mut self.buf));
            let mut buf = &self.buf[..count];
            while ! buf.is_empty() {
                state = match state {
                    ReadState::Waiting => {
                        let byte = buf[0];
                        buf = &buf[1..];
                        match byte {
                            raw_reply::ASYNC_UART_RX => ReadState::AsyncDataStart,
                            n if n >= raw_reply::ASYNC_PIN_CHANGE_N && n < (raw_reply::ASYNC_PIN_CHANGE_N + 16) => {
                                let pin = n & !(0x8);
                                let value = n & 0x8 != 0;
                                self.handle_pin_interrupt(pin, value);
                                ReadState::Waiting
                            },
                            raw_reply::HIGH | raw_reply::LOW => {
                                let request = self.requests.borrow_mut()
                                    .pop_back().expect("Got data without a request");
                                match request {
                                    ReadRequest::Pin(notify) => {
                                        notify.complete(byte == raw_reply::HIGH);
                                        ReadState::Waiting
                                    },
                                    _ => panic!("Expected pin value request"),
                                }
                            },
                            raw_reply::DATA => {
                                let request = self.requests.borrow_mut()
                                    .pop_back().expect("Got data without a request");
                                match request {
                                    ReadRequest::Data(count, notify) => {
                                        ReadState::ReplyData(vec![0; count].into_boxed_slice(), 0, notify)
                                    }
                                    _ => {
                                        panic!("Expected data request");
                                    }
                                }
                            },
                            _ => panic!("Unknown data recieved"),
                        }
                    }
                    ReadState::AsyncDataStart => {
                        let len = buf[0];
                        buf = &buf[1..];
                        ReadState::AsyncData(vec![0; len as usize].into_boxed_slice(), 0)
                    }
                    ReadState::AsyncData(mut data, mut offset) => {
                        let avail_len = cmp::min(data.len() - offset, buf.len());
                        data[offset..offset + avail_len].copy_from_slice(&buf[0..avail_len]);
                        offset += avail_len;
                        if offset == data.len() {
                            // Send future
                            ReadState::Waiting
                        } else {
                            ReadState::AsyncData(data, offset)
                        }
                    }
                    ReadState::ReplyData(mut data, mut offset, completion) => {
                        let avail_len = cmp::min(data.len() - offset, buf.len());
                        data[offset..offset + avail_len].copy_from_slice(&buf[0..avail_len]);
                        offset += avail_len;
                        if offset == data.len() {
                            completion.complete(data);
                            ReadState::Waiting
                        } else {
                            ReadState::ReplyData(data, offset, completion)
                        }
                    }
                }
            }
            self.state = state;
        }
    }
}

enum ReadState {
    Waiting,
    AsyncDataStart,
    AsyncData(Box<[u8]>, usize),
    ReplyData(Box<[u8]>, usize, Complete<Box<[u8]>>),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InterruptMode {
    Rise,
    Fall,
    Change,
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
    }
}
