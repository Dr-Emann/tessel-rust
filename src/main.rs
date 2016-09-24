extern crate tessel_raw;
extern crate tokio_core;
extern crate futures;

use futures::Future;
use futures::stream::Stream;

use tokio_core::reactor::{Core};

fn main() {
    let mut reactor = Core::new().unwrap();
    let port_a = tessel_raw::port::Port::connect("/tmp/tessel_sock.socket1",
                                               &reactor.handle()).unwrap();
    let port_b = tessel_raw::port::Port::connect("/tmp/tessel_sock.socket2",
                                               &reactor.handle()).unwrap();
    let port_a = reactor.run(port_a.into_i2c(100_000)).unwrap();
    let pin1 = port_b.pin(1);
    let future_a = port_a.read(1, 10);
    let future_b = pin1.write(true).and_then(|_| pin1.write(false)).and_then(|_| pin1.read());
    let pin_changes = pin1.changes().take(5).for_each(|b| {
        println!("pin changed to {}", b);
        Ok(())
    });

    let future = future_a.join3(future_b, pin_changes);
    
    println!("{:?}", reactor.run(future));
}
