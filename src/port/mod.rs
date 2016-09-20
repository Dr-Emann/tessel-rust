mod raw;
mod futures;
mod read_loop;

pub use self::futures::PinRead;
pub use self::futures::PinChanges;
pub use self::futures::I2cRead;
pub use self::futures::I2cPortFuture;


use std::io;
use std::path::Path;
use std::cell::RefCell;
use std::rc::Rc;
use std::collections::VecDeque;

use futures::{Complete, Future};
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
        let read_requests = Rc::new(RefCell::new(VecDeque::new()));
        let port = Port{
            handle: handle.clone(),
            stream: stream,
            callbacks: callbacks,
            read_requests: read_requests,
        };
        let reading = read_loop::Loop::new(&port);
        handle.spawn(reading.map_err(|e| panic!("{}", e)));
        Ok(port)
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

    pub fn into_i2c(self, frequency: u32) -> I2cPortFuture {
        I2cPortFuture::new(self, I2cPort::compute_baud(frequency))
    }

    pub fn i2c_read(&self, address: u8, len: u8) -> I2cRead {
        I2cRead::new(self, address, len)
    }
}

pub struct I2cPort {
    port: Port,
}

impl I2cPort {
    fn new(port: Port) -> I2cPort {
        I2cPort {
            port: port
        }
    }

    fn compute_baud(frequency: u32) -> u8 {
        let baud = 2.4e7 / (frequency as f64) - 5.36;
        if baud >= 0.0 && baud <= (u8::max_value() as f64) {
            baud as u8
        } else {
            panic!("invalid frequency");
        }
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

impl PinCallbacks {
    fn clear(&mut self) {
        *self = PinCallbacks::default()
    }
}

enum ReadRequest {
    Data(usize, Complete<Box<[u8]>>),
    Pin(Complete<bool>),
}
