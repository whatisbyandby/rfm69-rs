use crate::read_write::ReadWrite;
use crate::registers::Register;
use crate::settings::{
    ContinuousDagc, ModemConfigChoice, SyncConfiguration, RF69_FSTEP, RF69_FXOSC, RF_DIOMAPPING1_DIO0_00, RF_DIOMAPPING1_DIO0_01, RF_PALEVEL_OUTPUTPOWER_11111, RF_PALEVEL_PA0_ON, RF_PALEVEL_PA1_ON, RF_PALEVEL_PA2_ON
};
use embedded_hal::{delay::DelayNs, digital::OutputPin};

pub struct Rfm69<SPI, RESET, D> {
    pub spi: SPI,
    pub reset_pin: RESET,
    pub delay: D,
    tx_power: i8,
    current_mode: Rfm69Mode,
}

#[derive(Debug, PartialEq)]
pub enum Rfm69Error {
    ResetError,
    SpiWriteError,
    SpiReadError,
}

#[derive(Debug, PartialEq)]
pub enum Rfm69Mode {
    Sleep = 0x00,
    Standby = 0x04,
    Fs = 0x08,
    Tx = 0x0C,
    Rx = 0x10,
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
            tx_power: 13,
            current_mode: Rfm69Mode::Standby,
        }
    }

    pub fn init(&mut self) -> Result<(), Rfm69Error> {
        self.reset()?;

        let version = self.read_register(Register::Version)?;

        // the RFM69 module should return 0x24
        if version != 0x24 {
            return Err(Rfm69Error::SpiReadError);
        }

        self.set_default_fifo_threshold()?;
        self.set_dagc(ContinuousDagc::ImprovedLowBeta1)?;
        let sync_word = [0x2D, 0xD4];
        self.set_sync_words(
            SyncConfiguration::FifoFillAuto { sync_tolerance: 0 },
            &sync_word,
        )?;

        // If high power boost set previously, disable it
        self.write_register(Register::TestPa1, 0x55)?;
        self.write_register(Register::TestPa2, 0x70)?;

        self.set_modem_config(ModemConfigChoice::GfskRb250Fd250)?;

        self.set_preamble_length(4)?;

        self.set_frequency(915)?;

        Ok(())
    }

    pub fn read_all_registers(&mut self) -> Result<[(u8, u8); 84], Rfm69Error> {
        let mut registers = [0u8; 79];
        self.read_many(Register::OpMode, &mut registers)?;

        let mut mapped: [(u8, u8); 84] = [(0, 0); 84]; // Initialize the mapped array

        for (index, &value) in registers.iter().enumerate() {
            mapped[index] = ((index + 1).try_into().unwrap(), value);
        }

        mapped[79] = (0x58, self.read_register(Register::TestLna)?);
        mapped[80] = (0x5A, self.read_register(Register::TestPa1)?);
        mapped[81] = (0x5C, self.read_register(Register::TestPa2)?);
        mapped[82] = (0x6F, self.read_register(Register::TestDagc)?);
        mapped[83] = (0x71, self.read_register(Register::TestAfc)?);

        Ok(mapped)
    }

    pub fn read_revision(&mut self) -> Result<u8, Rfm69Error> {
        self.read_register(Register::Version)
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

        self.write_many(Register::DataModul, &values[0..5])?;
        self.write_many(Register::RxBw, &values[5..7])?;
        self.write_register(Register::PacketConfig1, values[7])?;

        Ok(())
    }

    fn set_preamble_length(&mut self, preamble_length: u16) -> Result<(), Rfm69Error> {
        // split the preamble length into two bytes
        let msb = (preamble_length >> 8) as u8;
        let lsb = preamble_length as u8;

        // write the two bytes to the RFM69
        let buffer = [msb, lsb];

        self.write_many(Register::PreambleMsb, &buffer)?;
        Ok(())
    }

    fn set_frequency(&mut self, freq_mhz: u32) -> Result<(), Rfm69Error> {
        let mut frf = (freq_mhz * RF69_FSTEP) as u32;
        frf /= RF69_FXOSC as u32;

        // split the frequency into three bytes
        let msb = ((frf >> 16) & 0xFF) as u8;
        let mid = ((frf >> 8) & 0xFF) as u8;
        let lsb = (frf & 0xFF) as u8;

        let buffer = [msb, mid, lsb];
        self.write_many(Register::FrfMsb, &buffer)?;
        Ok(())
    }

    fn set_tx_power(&mut self, tx_power: i8, is_high_power: bool) -> Result<(), Rfm69Error> {
        let pa_level;

        if is_high_power {
            let clamped_power = tx_power.clamp(-2, 20);

            if clamped_power <= 13 {
                // -2dBm to +13dBm
                // Need PA1 exclusivelly on RFM69HW
                pa_level =
                    RF_PALEVEL_PA1_ON | ((tx_power + 18) as u8 & RF_PALEVEL_OUTPUTPOWER_11111);
            } else if clamped_power >= 18 {
                // +18dBm to +20dBm
                // Need PA1+PA2
                // Also need PA boost settings change when tx is turned on and off, see setModeTx()
                pa_level = RF_PALEVEL_PA1_ON
                    | RF_PALEVEL_PA2_ON
                    | ((tx_power + 11) as u8 & RF_PALEVEL_OUTPUTPOWER_11111);
            } else {
                // +14dBm to +17dBm
                // Need PA1+PA2
                pa_level = RF_PALEVEL_PA1_ON
                    | RF_PALEVEL_PA2_ON
                    | ((tx_power + 14) as u8 & RF_PALEVEL_OUTPUTPOWER_11111);
            }
        } else {
            let clamped_power = tx_power.clamp(-18, 13);
            pa_level =
                RF_PALEVEL_PA0_ON | ((clamped_power + 18) as u8 & RF_PALEVEL_OUTPUTPOWER_11111);
        }

        self.write_register(Register::PaLevel, pa_level)?;
        self.tx_power = tx_power;
        Ok(())
    }

    fn set_mode(&mut self, mode: Rfm69Mode) -> Result<(), Rfm69Error> {
        if self.current_mode == mode {
            return Ok(());
        }

        match mode {
            Rfm69Mode::Rx => {
                // If high power boost, return power amp to receive mode
                if self.tx_power >= 18 {
                    self.write_register(Register::TestPa1, 0x55)?;
                    self.write_register(Register::TestPa2, 0x70)?;
                }
                // set DIOMAPPING1 to 0x01
                self.write_register(Register::DioMapping1, RF_DIOMAPPING1_DIO0_01)?;
            }

            Rfm69Mode::Tx => {
                // If high power boost, enable power amp
                if self.tx_power >= 18 {
                    self.write_register(Register::TestPa1, 0x5D)?;
                    self.write_register(Register::TestPa2, 0x7C)?;
                }

                self.write_register(Register::DioMapping1, RF_DIOMAPPING1_DIO0_00)?;
            }

            _ => {}
        }

        // Read the current mode
        let mut current_mode = self.read_register(Register::OpMode)?;
        current_mode &= !0x1C;
        current_mode |= mode as u8 & 0x1C;

        // // Set the new mode
        self.write_register(Register::OpMode, current_mode)?;
        while (self.read_register(Register::IrqFlags1)? & 0x80) == 0x00 {
            self.delay.delay_ms(10);
        }
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

    use embedded_hal::delay;
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

    #[test]
    fn test_set_preamble_length() {
        let mut rfm = setup_rfm();

        let spi_expectations = [
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::PreambleMsb.write()),
            SpiTransaction::write_vec(vec![0x00, 0xFF]),
            SpiTransaction::transaction_end(),
        ];

        rfm.spi.update_expectations(&spi_expectations);

        rfm.set_preamble_length(255).unwrap();

        check_expectations(&mut rfm);
    }

    #[test]
    fn test_get_revision() {
        let mut rfm = setup_rfm();

        let spi_expectations = [
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::Version.read()),
            SpiTransaction::transfer_in_place(vec![0x00], vec![0x24]),
            SpiTransaction::transaction_end(),
        ];

        rfm.spi.update_expectations(&spi_expectations);

        let revision = rfm.read_revision().unwrap();
        assert_eq!(revision, 0x24);

        check_expectations(&mut rfm);
    }

    #[test]
    fn test_set_frequency() {
        let mut rfm = setup_rfm();

        let spi_expectations = [
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::FrfMsb.write()),
            SpiTransaction::write_vec(vec![0xE4, 0xC0, 0x00]),
            SpiTransaction::transaction_end(),
        ];

        rfm.spi.update_expectations(&spi_expectations);

        rfm.set_frequency(915).unwrap();

        check_expectations(&mut rfm);
    }

    #[test]
    fn test_set_tx_power() {
        let mut rfm = setup_rfm();

        let spi_expectations = [
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::PaLevel.write()),
            SpiTransaction::write(0x50),
            SpiTransaction::transaction_end(),
        ];

        rfm.spi.update_expectations(&spi_expectations);

        rfm.set_tx_power(-2, true).unwrap();
        assert_eq!(rfm.tx_power, -2);

        check_expectations(&mut rfm);
    }

    #[test]
    fn test_set_mode_rx() {
        let mut rfm = setup_rfm();
        rfm.tx_power = 18;

        let spi_expectations = [
            // If high power boost, return power amp to receive mode
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::TestPa1.write()),
            SpiTransaction::write(0x55),
            SpiTransaction::transaction_end(),
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::TestPa2.write()),
            SpiTransaction::write(0x70),
            SpiTransaction::transaction_end(),
            // set DIOMAPPING1 to 0x01
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::DioMapping1.write()),
            SpiTransaction::write(RF_DIOMAPPING1_DIO0_01),
            SpiTransaction::transaction_end(),
            // Read the current value of OpMode
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::OpMode.read()),
            SpiTransaction::transfer_in_place(vec![0x00], vec![0xC4]),
            SpiTransaction::transaction_end(),
            // Set the new mode, leaving the other bits unchanged
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::OpMode.write()),
            SpiTransaction::write(0xD0),
            SpiTransaction::transaction_end(),
            // Wait for the mode to change
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::IrqFlags1.read()),
            SpiTransaction::transfer_in_place(vec![0x00], vec![0x00]),
            SpiTransaction::transaction_end(),
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::IrqFlags1.read()),
            SpiTransaction::transfer_in_place(vec![0x00], vec![0x80]),
            SpiTransaction::transaction_end(),
        ];

        let delay_expectations = [DelayTransaction::blocking_delay_ms(10)];

        rfm.spi.update_expectations(&spi_expectations);
        rfm.delay.update_expectations(&delay_expectations);

        rfm.set_mode(Rfm69Mode::Rx).unwrap();

        check_expectations(&mut rfm);
    }

    #[test]
    fn test_set_mode_tx() {
        let mut rfm = setup_rfm();
        rfm.tx_power = 18;

        let spi_expectations = [
            // If high power boost, return power amp to receive mode
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::TestPa1.write()),
            SpiTransaction::write(0x5D),
            SpiTransaction::transaction_end(),
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::TestPa2.write()),
            SpiTransaction::write(0x7C),
            SpiTransaction::transaction_end(),
            // set DIOMAPPING1 to 0x00
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::DioMapping1.write()),
            SpiTransaction::write(RF_DIOMAPPING1_DIO0_00),
            SpiTransaction::transaction_end(),
            // // Read the current value of OpMode
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::OpMode.read()),
            SpiTransaction::transfer_in_place(vec![0x00], vec![0xC4]),
            SpiTransaction::transaction_end(),
            // // Set the new mode, leaving the other bits unchanged
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::OpMode.write()),
            SpiTransaction::write(0xCC),
            SpiTransaction::transaction_end(),
            // // Wait for the mode to change
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::IrqFlags1.read()),
            SpiTransaction::transfer_in_place(vec![0x00], vec![0x00]),
            SpiTransaction::transaction_end(),
            SpiTransaction::transaction_start(),
            SpiTransaction::write(Register::IrqFlags1.read()),
            SpiTransaction::transfer_in_place(vec![0x00], vec![0x80]),
            SpiTransaction::transaction_end(),
        ];

        let delay_expectations = [
            DelayTransaction::blocking_delay_ms(10)
        ];

        rfm.spi.update_expectations(&spi_expectations);
        rfm.delay.update_expectations(&delay_expectations);

        rfm.set_mode(Rfm69Mode::Tx).unwrap();

        check_expectations(&mut rfm);
    }
}
