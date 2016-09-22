extern crate tessel_raw;
extern crate tokio_core;
extern crate futures;

use futures::Future;
use futures::stream::Stream;

use tokio_core::reactor::{Core};

fn main() {
    let mut reactor = Core::new().unwrap();
    let port = tessel_raw::port::Port::connect("/tmp/tessel_sock.socket",
                                               &reactor.handle()).unwrap();
    {
        let pin = port.pin(3);
        let future = pin.changes().take(5).for_each(|b| {
            println!("pin changed to {}", b);
            Ok(())
        });

        println!("{:?}", reactor.run(future));
    }
    let future = port.into_i2c(100_000);
    let i2c_port = reactor.run(future);

}
