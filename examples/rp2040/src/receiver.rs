#![no_std]
#![no_main]

use core::cell::RefCell;

use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::{gpio, peripherals::SPI0, spi::{self, Spi}};
use embassy_time::{Delay, Duration, Timer};
use gpio::{Level, Output};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

use embassy_embedded_hal::shared_bus::blocking::spi::SpiDevice;

use rfm69_rs::rfm69::Rfm69;
use embassy_sync::blocking_mutex::{raw::NoopRawMutex, Mutex};


type Spi0Bus = Mutex<NoopRawMutex, RefCell<Spi<'static, SPI0, spi::Blocking>>>;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let mut led = Output::new(p.PIN_27, Level::Low);

    let reset_pin = Output::new(p.PIN_20, Level::Low);
    let delay = Delay;

    let miso = p.PIN_16;
    let mosi = p.PIN_19;
    let clk = p.PIN_18;
    let cs_pin = p.PIN_17;

    let radio_cs = Output::new(cs_pin, Level::High);

    // create SPI
    let mut config = spi::Config::default();
    config.frequency = 1_000_000;
    config.phase = spi::Phase::CaptureOnFirstTransition;
    config.polarity = spi::Polarity::IdleLow;

    let spi = Spi::new_blocking(p.SPI0, clk, mosi, miso, config);

    let spi_ref = RefCell::new(spi);


    static SPI_BUS: StaticCell<Spi0Bus> = StaticCell::new();
    let spi_bus = SPI_BUS.init(Mutex::new(spi_ref));

    let spi_device = SpiDevice::new(spi_bus, radio_cs);


    
    let mut rfm69 = Rfm69::new(spi_device, reset_pin, delay);

    rfm69.init().unwrap();

    let registers = rfm69.read_all_registers().unwrap();
    registers.iter().for_each(|register| {
        info!("0x{:02X}: 0x{:02X}", register.0, register.1);
    });

    let temperature = rfm69.read_temperature().unwrap();
    info!("Temperature: {}", temperature);
    

    loop {
        rfm69.set_mode(rfm69_rs::rfm69::Rfm69Mode::Rx).unwrap();
        Timer::after(Duration::from_millis(10)).await;
        if rfm69.is_message_available().unwrap() {
            rfm69.set_mode(rfm69_rs::rfm69::Rfm69Mode::Standby).unwrap();
            led.set_high();
            
            let mut buffer = [0; 65];
            let message_length = rfm69.receive(&mut buffer).unwrap();

            let _received = core::str::from_utf8(&buffer[0..message_length]).is_ok_and(|message| {
                info!("Received message: {}", message);
                true
            });

            
            info!("Message Length: {:?}", message_length);
            let rssi = rfm69.rssi().unwrap();
            info!("RSSI: -{:?}", rssi);

            


            // give the transmitter some time to switch to RX
            Timer::after(Duration::from_millis(50)).await;

            // send ack
            rfm69.send("ACK".as_bytes()).unwrap();
            info!("Sent ACK");
            led.set_low();
        }
    }
    
}