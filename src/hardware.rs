// ==========================================
// HARDWARE I/O & INTERRUPTS
// ==========================================

use embassy_stm32::gpio::Input;
use embassy_time::{Duration, Timer};
use embassy_sync::watch::Watch;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use crate::app_state::UIEvent;

// MAX'S FEEDBACK: "Within the stm32 firmware you will get notified of button presses and encoder rotations using an embassy::sync::Watch."
// IMPLEMENTATION: Configured a global Watch channel to decouple hardware inputs from UI logic processing.
// NOTE: Using Option<UIEvent> to allow a Default initialization (None).
pub static UI_EVENT_CHANNEL: Watch<CriticalSectionRawMutex, Option<UIEvent>, 2> = Watch::new();

// MAX'S FEEDBACK / EMBASSY UPDATE FIX: "Type-erased EXTI channels (AnyChannel) are deprecated and unsafe."
// IMPLEMENTATION: Removed EXTI complexity. We now use a pure async polling system. 
// Uses generic `Input<'static>` to resolve macro duplication and type mismatch errors.
#[embassy_executor::task(pool_size = 7)]
pub async fn button_task(input: Input<'static>, event: UIEvent, log_name: &'static str) {
    loop {
        // 1. Yield CPU until button is pressed (Pin goes LOW due to Pull::Up)
        while input.is_high() {
            Timer::after(Duration::from_millis(10)).await;
        }
        
        // 2. Debounce delay (50ms is usually optimal for physical switches)
        Timer::after(Duration::from_millis(50)).await;
        
        // 3. Check if still pressed after debounce
        if input.is_low() {
            defmt::info!("[ACTION] {} Triggered!", log_name);
            
            // Send the event to the Watch channel to trigger the UI logic
            UI_EVENT_CHANNEL.sender().send(Some(event));
            
            // 4. Yield CPU until button is released (Prevents multi-triggering / auto-repeat)
            while input.is_low() {
                Timer::after(Duration::from_millis(10)).await;
            }
            
            // 5. Release debounce delay
            Timer::after(Duration::from_millis(50)).await;
            
            // FIX: Reset the watch channel to None!
            // Watch channels only trigger .changed() if the value is DIFFERENT.
            // Resetting to None ensures identical consecutive button presses are registered.
            UI_EVENT_CHANNEL.sender().send(None);
        }
    }
}