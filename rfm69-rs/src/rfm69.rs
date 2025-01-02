use crate::read_write::ReadWrite;
use crate::registers::Register;
use crate::settings::{ContinuousDagc, ModemConfigChoice, SyncConfiguration};
use embedded_hal::{delay::DelayNs, digital::OutputPin};

pub struct Rfm69<SPI, RESET, D> {
    pub spi: SPI,
    pub reset_pin: RESET,
    pub delay: D,
}

#[derive(Debug, PartialEq)]
pub enum Rfm69Error {
    ResetError,
    SpiWriteError,
    SpiReadError,
}

impl<SPI, RESET, D> Rfm69<SPI, RESET, D>
where
    SPI: ReadWrite,
    RESET: OutputPin,
    D: DelayNs,
{
    fn reset(&mut self) -> Result<(), Rfm69Error> {
        self.reset_pin
            .set_high()
            .map_err(|_| Rfm69Error::ResetError)?;
        self.delay.delay_ms(10);
        self.reset_pin
            .set_low()
            .map_err(|_| Rfm69Error::ResetError)?;
        self.delay.delay_ms(10);
        Ok(())
    }

    pub fn new(spi: SPI, reset_pin: RESET, delay: D) -> Self {
        Rfm69 {
            spi,
            reset_pin,
            delay,
        }
    }

    pub fn init(&mut self) -> Result<(), Rfm69Error> {
        self.reset()?;

        self.set_default_fifo_threshold()?;
        self.set_dagc(ContinuousDagc::ImprovedLowBeta1)?;
        let sync_word = [0x2D, 0xD4];
        self.set_sync_words(
            SyncConfiguration::FifoFillAuto { sync_tolerance: 0 },
            &sync_word,
        )?;

        Ok(())
    }

    pub fn read_temperature(&mut self) -> Result<f32, Rfm69Error> {
        self.write_register(Register::Temp1, 0x08)?;
        while self.read_register(Register::Temp1)? & 0x04 != 0x00 {
            self.delay.delay_ms(10);
        }

        let temp = self.read_register(Register::Temp2)?;
        Ok((166 as f32) - temp as f32)
    }

    fn set_default_fifo_threshold(&mut self) -> Result<(), Rfm69Error> {
        self.write_register(Register::FifoThresh, 0x8F)?;
        Ok(())
    }

    fn set_dagc(&mut self, value: ContinuousDagc) -> Result<(), Rfm69Error> {
        self.write_register(Register::TestDagc, value as u8)?;
        Ok(())
    }

    fn set_sync_words(
        &mut self,
        config: SyncConfiguration,
        sync_words: &[u8],
    ) -> Result<(), Rfm69Error> {
        if sync_words.len() > 8 || sync_words.len() == 0 {
            return Err(Rfm69Error::SpiWriteError);
        }

        let mut buffer = [0u8; 9]; // 1 byte for config + up to 8 bytes for sync words

        // Add the config value to the first position
        // We need to know how many sync words we have to set the correct config value
        buffer[0] = config.value(sync_words.len() as u8);
        // Add the sync words to the buffer
        buffer[1..1 + sync_words.len()].copy_from_slice(sync_words);
        // Write the config value first, then the sync words.
        self.write_many(Register::SyncConfig, &buffer)?;

        Ok(())
    }

    fn set_modem_config(&mut self, config: ModemConfigChoice) -> Result<(), Rfm69Error> {
        let values = config.values();

        // write the first 5 bytes in the values array

        self.write_many(Register::DataModul, &values[0..5])?;
        self.write_many(Register::RxBw, &values[5..7])?;
        self.write_register(Register::PacketConfig1, values[7])?;

        Ok(())
    }

    fn write_register(&mut self, register: Register, value: u8) -> Result<(), Rfm69Error> {
        self.write_many(register, &[value])?;
        Ok(())
    }

    fn read_register(&mut self, register: Register) -> Result<u8, Rfm69Error> {
        let mut buffer = [0u8; 1];
        self.spi
            .read_many(register, &mut buffer)
            .map_err(|_| Rfm69Error::SpiWriteError)?;
        Ok(buffer[0])
    }

    fn write_many(&mut self, register: Register, values: &[u8]) -> Result<(), Rfm69Error> {
        self.spi
            .write_many(register, values)
            .map_err(|_| Rfm69Error::SpiWriteError)?;
        Ok(())
    }

    fn read_many(&mut self, register: Register, buffer: &mut [u8]) -> Result<(), Rfm69Error> {
        self.spi
            .read_many(register, buffer)
            .map_err(|_| Rfm69Error::SpiReadError)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use crate::settings::{ContinuousDagc, SyncConfiguration};

    use super::*;

    use embedded_hal_mock::eh1::delay::{CheckedDelay, Transaction as DelayTransaction};
    use embedded_hal_mock::eh1::digital::{
        Mock as DigitalMock, State, Transaction as GpioTransaction,
    };
    use embedded_hal_mock::eh1::spi::{Mock as SpiDevice, Transaction as SpiTransaction};

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
            SpiTransaction::write(Register::Temp1.read()),
            SpiTransaction::transfer_in_place(vec![0x00], vec![0x04]),
            SpiTransaction::transaction_end(),
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::Temp1.read()),
            SpiTransaction::transfer_in_place(vec![0x00], vec![0x00]),
            SpiTransaction::transaction_end(),
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::Temp2.read()),
            SpiTransaction::transfer_in_place(vec![0x00], vec![0x8D]),
            SpiTransaction::transaction_end(),
        ];
        rfm.spi.update_expectations(&temperature_expectations);

        let delay_expectations = [DelayTransaction::blocking_delay_ms(10)];
        rfm.delay.update_expectations(&delay_expectations);

        let temperature = rfm.read_temperature().unwrap();

        assert_eq!(temperature, 25.0);

        check_expectations(&mut rfm);
    }

    #[test]
    fn test_set_default_fifo_threshold() {
        let mut rfm = setup_rfm();

        let spi_expectations = [
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::FifoThresh.write()),
            SpiTransaction::write(0x8F),
            SpiTransaction::transaction_end(),
        ];

        rfm.spi.update_expectations(&spi_expectations);

        rfm.set_default_fifo_threshold().unwrap();

        check_expectations(&mut rfm);
    }

    #[test]
    fn test_set_dagc() {
        let mut rfm = setup_rfm();

        let spi_expectations = [
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::TestDagc.write()),
            SpiTransaction::write(ContinuousDagc::ImprovedLowBeta1 as u8),
            SpiTransaction::transaction_end(),
        ];

        rfm.spi.update_expectations(&spi_expectations);

        rfm.set_dagc(ContinuousDagc::ImprovedLowBeta1).unwrap();

        check_expectations(&mut rfm);
    }

    #[test]
    fn test_set_power() {
        let mut rfm = setup_rfm();

        let spi_expectations = [];

        rfm.spi.update_expectations(&spi_expectations);

        check_expectations(&mut rfm);
    }

    #[test]
    fn test_set_sync_words() {
        let mut rfm = setup_rfm();

        let spi_expectations = [
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::SyncConfig.write()),
            SpiTransaction::write_vec(vec![184, 1, 2, 3, 4, 5, 6, 7, 8]),
            SpiTransaction::transaction_end(),
        ];

        rfm.spi.update_expectations(&spi_expectations);

        let sync_words = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        rfm.set_sync_words(
            SyncConfiguration::FifoFillAuto { sync_tolerance: 0 },
            &sync_words,
        )
        .unwrap();

        check_expectations(&mut rfm);
    }

    #[test]
    fn test_set_sync_words_clamp() {
        let mut rfm = setup_rfm();

        let spi_expectations = [
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::SyncConfig.write()),
            SpiTransaction::write_vec(vec![191, 1, 2, 3, 4, 5, 6, 7, 8]),
            SpiTransaction::transaction_end(),
        ];

        rfm.spi.update_expectations(&spi_expectations);

        let sync_words = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];

        // sync_tolerance is clamped to 7
        rfm.set_sync_words(
            SyncConfiguration::FifoFillAuto { sync_tolerance: 14 },
            &sync_words,
        )
        .unwrap();

        check_expectations(&mut rfm);
    }

    #[test]
    fn test_set_sync_words_too_long() {
        let mut rfm = setup_rfm();

        let spi_expectations = [];

        rfm.spi.update_expectations(&spi_expectations);

        let sync_words = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09];
        assert_eq!(
            rfm.set_sync_words(
                SyncConfiguration::FifoFillAuto { sync_tolerance: 0 },
                &sync_words
            ),
            Err(Rfm69Error::SpiWriteError)
        );

        check_expectations(&mut rfm);
    }

    #[test]
    fn test_set_sync_words_empty() {
        let mut rfm = setup_rfm();

        let spi_expectations = [];

        rfm.spi.update_expectations(&spi_expectations);

        let sync_words = [];
        assert_eq!(
            rfm.set_sync_words(
                SyncConfiguration::FifoFillAuto { sync_tolerance: 0 },
                &sync_words
            ),
            Err(Rfm69Error::SpiWriteError)
        );

        check_expectations(&mut rfm);
    }

    #[test]
    fn test_set_modem_config() {
        let mut rfm = setup_rfm();

        let spi_expectations = [
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::DataModul.write()),
            SpiTransaction::write_vec(vec![0x00, 0x3e, 0x80, 0x00, 0x52]),
            SpiTransaction::transaction_end(),
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::RxBw.write()),
            SpiTransaction::write_vec(vec![0xf4, 0xf4]),
            SpiTransaction::transaction_end(),
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::PacketConfig1.write()),
            SpiTransaction::write(0xd0),
            SpiTransaction::transaction_end(),
        ];

        rfm.spi.update_expectations(&spi_expectations);

        rfm.set_modem_config(ModemConfigChoice::FskRb2Fd5).unwrap();

        check_expectations(&mut rfm);
    }
}
