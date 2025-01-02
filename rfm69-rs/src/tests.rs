
mod tests {
    
    use embedded_hal::spi::SpiBus;
    use embedded_hal_mock::eh1::delay::{CheckedDelay, Transaction as DelayTransaction};
    use embedded_hal_mock::eh1::digital::{Mock as DigitalMock, State, Transaction as GpioTransaction};
    use embedded_hal_mock::eh1::spi::{Mock as SpiDevice, Transaction as SpiTransaction};

    use crate::rfm69::Rfm69;


    fn setup_rfm() -> Rfm69<SpiDevice<u8>, DigitalMock, CheckedDelay> {
        let spi_expectations = [];
        let spi_device = SpiDevice::new(spi_expectations);

        let reset_expectations = [];
        let reset_pin = DigitalMock::new(reset_expectations);

        let delay_expectations = [];
        let delay = CheckedDelay::new(delay_expectations);

        Rfm69::new(spi_device, reset_pin, delay)
    }

    fn check_expectations(rfm: &mut Rfm69<SpiDevice<u8>, DigitalMock, CheckedDelay>) {
        rfm.reset_pin.done();
        rfm.delay.done();
        rfm.spi.done();
    }


    #[test]
    fn test_rfm69_reset() {


        let mut rfm = setup_rfm();

        let reset_expectations = [
            GpioTransaction::set(State::High),
            GpioTransaction::set(State::Low),
        ];
        rfm.reset_pin.update_expectations(&reset_expectations);

        let delay_expectations = [
            DelayTransaction::blocking_delay_ms(10),
            DelayTransaction::blocking_delay_ms(10),
        ];
        rfm.delay.update_expectations(&delay_expectations);

        rfm.reset().unwrap();

        check_expectations(&mut rfm);
    }

    #[test]
    fn test_rfm69_read_temperature() {
        let mut rfm = setup_rfm();

        let temperature_expectations = [
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::Temp1.write()),
            SpiTransaction::write(0x08),
            SpiTransaction::transaction_end(),
            SpiTransaction::transaction_start(),
            SpiTransaction::transfer_in_place(vec![Register::Temp1.read()], vec![0x04]),
            SpiTransaction::transaction_end(),
            SpiTransaction::transaction_start(),
            SpiTransaction::transfer_in_place(vec![Register::Temp2.read()], vec![0x0A]),
            SpiTransaction::transaction_end(),
        ];

        rfm.spi.update_expectations(&temperature_expectations);

        let temperature = rfm.read_temperature().unwrap();

        check_expectations(&mut rfm);

    }
}
