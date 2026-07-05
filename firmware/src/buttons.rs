use embassy_executor;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::{gpio::Input, rng::Rng};
use log::error;
use smart_leds_trait::{RGB8, SmartLedsWrite};
use ws2812_timer_delay as ws2812;

use crate::state::AppState;
use crate::state_cell::StateCell;
use crate::ws2812_impl::{Ws2812Pin, Ws2812Timer};

/// Window in which the player must press the lit button to avoid damage.
const REACTION_WINDOW: Duration = Duration::from_millis(800);
/// Once the window expires, damage is applied every `DAMAGE_TICK` and the
/// amount doubles on each tick (exponential drain).
const DAMAGE_TICK: Duration = Duration::from_millis(150);

/// Per-LED colours. Button `n` is paired with `LED_COLORS[n]`.
const LED_COLORS: [RGB8; 4] = [
    RGB8::new(255, 0, 0),
    RGB8::new(0, 255, 0),
    RGB8::new(0, 0, 255),
    RGB8::new(255, 255, 0),
];

/// The four reaction buttons, each mapped to one LED. Pulled up so that an
/// external button to ground reads as `Level::Low` when pressed.
pub struct Buttons {
    pub btn0: Input<'static>,
    pub btn1: Input<'static>,
    pub btn2: Input<'static>,
    pub btn3: Input<'static>,
}

/// Reaction mini-game: lights a random LED and drains the player's HP if the
/// matching button isn't pressed in time. Damage doubles every tick past the
/// reaction window until the button is finally pressed.
#[embassy_executor::task]
pub async fn button_task(
    state: &'static StateCell<CriticalSectionRawMutex, AppState, 1>,
    mut leds: ws2812::Ws2812<Ws2812Timer, Ws2812Pin>,
    buttons: Buttons,
) {
    let rng = Rng::new();
    let off = [RGB8::new(0, 0, 0); 4];

    // Wait until the game phase begins.
    loop {
        let s = state.get().await;
        if matches!(s, AppState::Game { .. }) {
            break;
        }
        Timer::after_millis(50).await;
    }

    loop {
        let current = state.get().await;
        if !matches!(current, AppState::Game { .. }) {
            let _ = leds.write(off.iter().copied());
            return;
        }

        // Light up a random LED and remember when it turned on.
        let idx = (rng.random() as usize) % 4;
        let mut buf = off;
        buf[idx] = LED_COLORS[idx];
        if leds.write(buf.iter().copied()).is_err() {
            error!("ws2812 write failed");
        }
        let lit_at = Instant::now();

        // Exponential damage multiplier, reset for each new LED. The first
        // tick of damage (once the reaction window elapses) drains 1 HP; each
        // subsequent tick drains double the previous amount.
        let mut damage: u32 = 1;

        // Spin until the matching button is pressed. While inside the reaction
        // window the player takes no damage. Once it expires we drain HP every
        // `DAMAGE_TICK`, doubling the amount each tick until the button is
        // finally pressed.
        loop {
            let pressed = match idx {
                0 => buttons.btn0.is_low(),
                1 => buttons.btn1.is_low(),
                2 => buttons.btn2.is_low(),
                _ => buttons.btn3.is_low(),
            };
            if pressed {
                break;
            }

            if lit_at.elapsed() > REACTION_WINDOW {
                state
                    .update(|s| match s {
                        AppState::Game {
                            soup_hp,
                            player_hp,
                            soup_status,
                            sweet_spot_min,
                            sweet_spot_max,
                            sweet_spot_progress,
                            loadcell_reading,
                        } => AppState::Game {
                            soup_hp: *soup_hp,
                            player_hp: player_hp.saturating_sub(damage),
                            soup_status: soup_status.clone(),
                            sweet_spot_min: *sweet_spot_min,
                            sweet_spot_max: *sweet_spot_max,
                            sweet_spot_progress: *sweet_spot_progress,
                            loadcell_reading: *loadcell_reading,
                        },
                        other => other.clone(),
                    })
                    .await;
                let after = state.get().await;
                if let AppState::Game { player_hp: 0, .. } = after {
                    let _ = leds.write(off.iter().copied());
                    state.set(AppState::EndScreen { player_won: false }).await;
                    return;
                }
                damage = damage.saturating_mul(2);
                Timer::after(DAMAGE_TICK).await;
            } else {
                // Still inside the reaction window: poll quickly.
                Timer::after_millis(5).await;
            }
        }

        // Button pressed: turn the LED off and wait a random beat before
        // lighting the next one.
        let _ = leds.write(off.iter().copied());
        let gap_ms = 300 + (rng.random() % 700);
        Timer::after_millis(gap_ms as u64).await;
    }
}
