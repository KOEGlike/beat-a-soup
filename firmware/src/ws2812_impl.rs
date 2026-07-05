//! Adapters that bridge esp-hal 1.x peripherals to the `embedded-hal` 0.2
//! trait surface expected by `ws2812-timer-delay`.
//!
//! `ws2812-timer-delay` drives a WS2812 strip by bit-banging a GPIO pin and
//! pacing each bit with a periodic timer that ticks at 3 MHz. It requires the
//! timer to implement `embedded_hal::timer::{CountDown, Periodic}` (the 0.2
//! traits) and the data pin to implement
//! `embedded_hal::digital::v2::OutputPin` (also 0.2).
//!
//! esp-hal 1.x only exposes the 1.0 `embedded-hal` traits, and its
//! `time::Duration` is microsecond-granular so the public `PeriodicTimer` API
//! cannot express a 333 ns period. To get a real 3 MHz tick we therefore drive
//! `TIMG0` Timer0 directly through the PAC register block: the APB clock runs
//! at 80 MHz, the minimum prescaler divides by 2 (40 MHz tick = 25 ns), and a
//! load/alarm value of 13 gives a 325 ns period (~3.08 MHz).
//!
//! The timer is owned by the `Ws2812Timer` zero-sized handle; `TIMG0`'s
//! peripheral clock must already be enabled (done in `main` via
//! `TimerGroup::new(p.TIMG0)`) before `Ws2812Timer::new` is called.

use embedded_hal_02::{
    digital::v2::OutputPin as OutputPin02,
    timer::{CountDown as CountDown02, Periodic as Periodic02},
};
use esp_hal::{gpio::Output, peripherals::TIMG0};
use void::Void;

/// Number of 40 MHz ticks per WS2812 bit-cell third.
///
/// 13 ticks * 25 ns = 325 ns, i.e. a ~3.08 MHz tick as expected by
/// `ws2812-timer-delay`. Each bit cell consists of 3 `wait()` periods, giving
/// a ~975 ns bit (~1.03 MHz), well inside the WS2812 timing window.
pub const WS2812_TICKS: u32 = 13;

/// A periodic 3 MHz countdown timer backed by `TIMG0` Timer0, exposing the
/// `embedded-hal` 0.2 `CountDown` + `Periodic` traits.
pub struct Ws2812Timer;

impl Ws2812Timer {
    /// Configure and start `TIMG0` Timer0 as a periodic timer with the given
    /// alarm value (in 40 MHz ticks).
    ///
    /// The `TIMG0` peripheral clock must be enabled first (e.g. via
    /// `TimerGroup::new(p.TIMG0)`), and the TIMG0 watchdog should be disabled
    /// to avoid spurious resets.
    pub fn new(period_ticks: u32) -> Self {
        let regs = TIMG0::regs();
        let t = regs.t(0);

        // Stop the timer and its alarm before reconfiguring.
        t.config()
            .modify(|_, w| w.en().bit(false).alarm_en().bit(false));
        // Drop any pending interrupt.
        regs.int_clr().write(|w| w.t0().clear_bit_by_one());

        // Counter starts at 0 (up-counting mode); alarm fires at `period_ticks`.
        // SAFETY: writing raw bit patterns to MMIO register fields.
        unsafe {
            t.loadlo().write(|w| w.load_lo().bits(0));
            t.loadhi().write(|w| w.load_hi().bits(0));
            t.load().write(|w| w.load().bits(1));
            t.alarmlo().write(|w| w.alarm_lo().bits(period_ticks));
            t.alarmhi().write(|w| w.alarm_hi().bits(0));
        }

        // Up-counting, auto-reload, divide APB (80 MHz) by 2 -> 40 MHz tick,
        // sourced from APB (not XTAL).
        // SAFETY: `.bits()` writes a raw field value to an MMIO register.
        t.config().modify(|_, w| unsafe {
            w.increase()
                .bit(true)
                .autoreload()
                .bit(true)
                .use_xtal()
                .bit(false)
                .divider()
                .bits(2)
        });

        // Enable the counter and the alarm.
        t.config()
            .modify(|_, w| w.en().bit(true).alarm_en().bit(true));

        Self
    }
}

impl CountDown02 for Ws2812Timer {
    type Time = u32;

    fn start<T>(&mut self, count: T)
    where
        T: Into<Self::Time>,
    {
        let ticks = count.into();
        let regs = TIMG0::regs();
        let t = regs.t(0);
        t.config()
            .modify(|_, w| w.en().bit(false).alarm_en().bit(false));
        regs.int_clr().write(|w| w.t0().clear_bit_by_one());
        // SAFETY: writing raw bit patterns to MMIO register fields.
        unsafe {
            t.alarmlo().write(|w| w.alarm_lo().bits(ticks));
            t.alarmhi().write(|w| w.alarm_hi().bits(0));
        }
        t.config()
            .modify(|_, w| w.en().bit(true).alarm_en().bit(true));
    }

    fn wait(&mut self) -> nb::Result<(), Void> {
        let regs = TIMG0::regs();
        while !regs.int_raw().read().t0().bit_is_set() {}
        regs.int_clr().write(|w| w.t0().clear_bit_by_one());
        Ok(())
    }
}

impl Periodic02 for Ws2812Timer {}

/// Newtype around an esp-hal `Output` pin exposing the `embedded-hal` 0.2
/// `OutputPin` trait required by the WS2812 driver.
pub struct Ws2812Pin(pub Output<'static>);

impl OutputPin02 for Ws2812Pin {
    type Error = core::convert::Infallible;

    fn set_high(&mut self) -> Result<(), Self::Error> {
        self.0.set_high();
        Ok(())
    }

    fn set_low(&mut self) -> Result<(), Self::Error> {
        self.0.set_low();
        Ok(())
    }
}
