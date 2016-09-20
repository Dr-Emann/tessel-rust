#[allow(dead_code)]
pub mod cmd {
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
pub mod reply {
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

#[allow(dead_code)]
pub mod interrupt_mode {
    pub const RISE: u8 = 1;
    pub const FALL: u8 = 2;
    pub const CHANGE: u8 = 3;
    pub const HIGH: u8 = 4;
    pub const LOW: u8 = 5;
}
