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
pub static UI_EVENT_CHANNEL: Watch<CriticalSectionRawMutex, Option<UIEvent>, 2> = Watch::new();

// MAX'S FEEDBACK / EMBASSY UPDATE FIX: "Type-erased EXTI channels (AnyChannel) are deprecated and unsafe."
// IMPLEMENTATION: Removed EXTI complexity. We now use a pure async polling system. 
// Uses generic `Input<'static>` to resolve macro duplication and type mismatch errors.
#[embassy_executor::task(pool_size = 7)]
pub async fn button_task(input: Input<'static>, event: UIEvent, log_name: &'static str) {
    loop {
        // Yield CPU until button is pressed (Pin goes LOW)
        while input.is_high() {
            Timer::after(Duration::from_millis(10)).await;
        }
        
        // Debounce delay (100ms for stable hardware switch reading)
        Timer::after(Duration::from_millis(100)).await;
        
        // Check if still pressed after debounce
        if input.is_low() {
            defmt::info!("[ACTION] {} Triggered!", log_name);
            UI_EVENT_CHANNEL.sender().send(Some(event));
            
            // Yield CPU until button is released to prevent multi-triggering
            while input.is_low() {
                Timer::after(Duration::from_millis(10)).await;
            }
            Timer::after(Duration::from_millis(100)).await; // Release debounce
        }
    }
}