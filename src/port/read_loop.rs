use std::{cmp, fmt, mem, io};
use std::io::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::collections::VecDeque;

use futures::{Future, Poll, Complete};
use tokio_uds::UnixStream;

use super::{Port, ReadRequest, PinCallbacks};
use super::raw;

pub struct Loop {
    stream: Rc<UnixStream>,
    callbacks: Rc<RefCell<[PinCallbacks; 8]>>,
    requests: Rc<RefCell<VecDeque<ReadRequest>>>,
    state: ReadState,
    buf: [u8; 1024],
}

impl Loop {
    pub fn new(port: &Port) -> Loop {
        Loop {
            stream: port.stream.clone(),
            callbacks: port.callbacks.clone(),
            requests: port.read_requests.clone(),
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

impl Future for Loop {
    type Item = ();
    type Error = io::Error;
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            let count = match (&*self.stream).read(&mut self.buf) {
                Ok(t) => t,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    println!("Err Block");
                    return Ok(::futures::Async::NotReady)
                }
                Err(e) => {
                    return Err(e.into());
                }
            };
            if count == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Socket closed"));
            }

            let mut buf = &self.buf[..count];
            let mut state = mem::replace(&mut self.state, ReadState::Missing);
            while ! buf.is_empty() {
                state = match state {
                    ReadState::Missing => panic!("Port reader in invalid state"),
                    ReadState::Waiting => {
                        let byte = buf[0];
                        buf = &buf[1..];
                        match byte {
                            raw::reply::ASYNC_UART_RX => ReadState::AsyncDataStart,
                            n if n >= raw::reply::ASYNC_PIN_CHANGE_N && n < (raw::reply::ASYNC_PIN_CHANGE_N + 16) => {
                                let pin = (n - raw::reply::ASYNC_PIN_CHANGE_N) & !(0x8);
                                let value = n & 0x8 != 0;
                                self.handle_pin_interrupt(pin, value);
                                ReadState::Waiting
                            },
                            raw::reply::HIGH | raw::reply::LOW => {
                                let request = self.requests.borrow_mut()
                                    .pop_back().expect("Got data without a request");
                                match request {
                                    ReadRequest::Pin(notify) => {
                                        notify.complete(byte == raw::reply::HIGH);
                                        ReadState::Waiting
                                    },
                                    _ => panic!("Expected pin value request"),
                                }
                            },
                            raw::reply::DATA => {
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
                        buf = &buf[avail_len..];
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
                        buf = &buf[avail_len..];
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

impl Drop for Loop {
    fn drop(&mut self) {
        self.requests.borrow_mut().clear();
        let mut callbacks = self.callbacks.borrow_mut();
        for cb in callbacks.iter_mut() {
            cb.clear();
        }
    }
}

enum ReadState {
    Missing,
    Waiting,
    AsyncDataStart,
    AsyncData(Box<[u8]>, usize),
    ReplyData(Box<[u8]>, usize, Complete<Box<[u8]>>),
}

impl fmt::Debug for ReadState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::ReadState::*;
        match *self {
            Missing => write!(f, "Missing"),
            Waiting => write!(f, "Waiting"),
            AsyncDataStart => write!(f, "AsyncDataStart"),
            AsyncData(ref data, len) => write!(f, "AsyncData({} of {})", len, data.len()),
            ReplyData(ref data, len, _) => write!(f, "ReplyData({} of {})", len, data.len()),
        }
    }
}
