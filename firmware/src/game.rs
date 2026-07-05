use embassy_executor;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::Timer;

use crate::state::{AppState, SoupStatus};
use crate::state_cell::StateCell;

/// Drives the top-level game flow: hold the start screen, show the rules, then
/// drop into the active `Game` state. Win/lose transitions are handled by the
/// button and load-cell tasks.
#[embassy_executor::task]
pub async fn logic_task(state: &'static StateCell<CriticalSectionRawMutex, AppState, 1>) {
    state.set(AppState::Start).await;
    Timer::after_secs(20).await;
    state.set(AppState::Rules).await;
    Timer::after_secs(30).await;
    state
        .set(AppState::Game {
            soup_hp: 100,
            player_hp: 100,
            soup_status: SoupStatus::Neutral,
            sweet_spot_min: 0,
            sweet_spot_max: 0,
            sweet_spot_progress: 0,
            loadcell_reading: 0,
        })
        .await;
}
