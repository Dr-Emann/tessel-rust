mod raw;
mod futures;

pub use self::futures::PinRead;
pub use self::futures::PinChanges;
pub use self::futures::I2cRead;

use std::{io, cmp, mem};
use std::io::prelude::*;
use std::path::Path;
use std::cell::RefCell;
use std::rc::Rc;
use std::collections::VecDeque;

use futures::{Complete, Future, Poll};
use tokio_uds::UnixStream;
use tokio_core::io as tokio_io;
use tokio_core::reactor::Handle;
use tokio_core::channel::Sender;

pub struct Port {
    handle: Handle,
    stream: Rc<UnixStream>,
    callbacks: Rc<RefCell<[PinCallbacks; 8]>>,
    read_requests: Rc<RefCell<VecDeque<ReadRequest>>>,
}

impl Port {
    pub fn connect<P: AsRef<Path> + ?Sized>(path: &P, handle: &Handle) -> io::Result<Port> {
        Port::_connect(path.as_ref(), handle)
    }

    fn _connect(path: &Path, handle: &Handle) -> io::Result<Port> {
        let stream = Rc::new(try!(UnixStream::connect(path, handle)));
        let callbacks = Rc::new(RefCell::new(Default::default()));
        let requests = Rc::new(RefCell::new(VecDeque::new()));
        let reading = ReadingFuture::new(stream.clone(), callbacks.clone(), requests.clone());
        handle.spawn(reading.map_err(|e| panic!("{}", e)));
        Ok(Port{
            handle: handle.clone(),
            stream: stream,
            callbacks: callbacks.clone(),
            read_requests: requests,
        })
    }

    pub fn gpio_digital_write(&self, pin: u8, value: bool) -> tokio_io::WriteAll<&UnixStream, [u8; 2]> {
        let cmd = if value { raw::cmd::GPIO_HIGH } else { raw::cmd::GPIO_LOW };
        tokio_io::write_all(&*self.stream, [cmd, pin])
    }

    pub fn gpio_digital_read(&self, pin: u8) -> PinRead {
        PinRead::new(self, pin)
    }

    pub fn pin_changes(&self, pin: u8) -> PinChanges {
        PinChanges::new(self, pin)
    }

    pub fn i2c_read(&self, address: u8, len: u8) -> I2cRead {
        I2cRead::new(self, address, len)
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

struct ReadingFuture {
    stream: Rc<UnixStream>,
    callbacks: Rc<RefCell<[PinCallbacks; 8]>>,
    requests: Rc<RefCell<VecDeque<ReadRequest>>>,
    state: ReadState,
    buf: [u8; 1024],
}

impl ReadingFuture {
    fn new(stream: Rc<UnixStream>, callbacks: Rc<RefCell<[PinCallbacks; 8]>>,
           requests: Rc<RefCell<VecDeque<ReadRequest>>>) -> ReadingFuture {
        ReadingFuture {
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

impl Future for ReadingFuture {
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
                            raw::reply::ASYNC_UART_RX => ReadState::AsyncDataStart,
                            n if n >= raw::reply::ASYNC_PIN_CHANGE_N && n < (raw::reply::ASYNC_PIN_CHANGE_N + 16) => {
                                let pin = n & !(0x8);
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

enum ReadRequest {
    Data(usize, Complete<Box<[u8]>>),
    Pin(Complete<bool>),
}

enum ReadState {
    Waiting,
    AsyncDataStart,
    AsyncData(Box<[u8]>, usize),
    ReplyData(Box<[u8]>, usize, Complete<Box<[u8]>>),
}
