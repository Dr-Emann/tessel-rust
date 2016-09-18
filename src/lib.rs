extern crate futures;
#[macro_use]
extern crate tokio_core;
extern crate tokio_uds;

pub mod port;

use std::io;

use tokio_core::reactor::Handle;

use port::Port;

const ANALOG_RESOLUTION: u16 = 0x1000;

const PORT_A_PATH: &'static str = "/var/run/tessel/port_a";
const PORT_B_PATH: &'static str = "/var/run/tessel/port_b";

pub struct Tessel {
    port_a: Port,
    port_b: Port,
}

impl Tessel {
    pub fn new(handle: &Handle) -> io::Result<Tessel> {
        let port_a = try!(Port::connect(PORT_A_PATH, handle));
        let port_b = try!(Port::connect(PORT_B_PATH, handle));
        Ok(Tessel {
            port_a: port_a,
            port_b: port_b,
        })
    }
}
