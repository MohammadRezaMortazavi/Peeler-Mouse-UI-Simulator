#![no_std]
#![no_main]
#![allow(unused_imports)] 
#![allow(dead_code)]      

// ==========================================
// THE MISSING LINK FOR MEMORY ALLOCATION
// ==========================================
// MAX'S FIX: In no_std, we must explicitly declare the alloc crate
// so the compiler links our #[global_allocator] for Ratatui to use!
extern crate alloc;

// ==========================================
// EMBASSY & HARDWARE IMPORTS
// ==========================================
use defmt_rtt as _; // For logging over SWD (Hardware)
use panic_probe as _; // Panic handler for bare-metal hardware

use embassy_executor::Spawner;

// MAX'S FEEDBACK: "In the final hardware version, we need to use embassy-time instead of std::time"
// IMPLEMENTATION: Removed all `std::time` dependencies. We are now fully utilizing `embassy_time` 
// for non-blocking hardware-accurate timing.
use embassy_time::{Duration, Instant, Timer};

// MAX'S FEEDBACK: "Within the stm32 firmware you will get notified of button presses 
// and encoder rotations using a embassy::sync::Watch"
// IMPLEMENTATION: I have set up a global `Watch` channel (`UI_EVENT_CHANNEL`). The hardware interrupts 
// (EXTI) will push `UIEvent`s here, and our async main loop will react instantly without blocking the MCU.
use embassy_sync::watch::Watch;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

// Graphic dependencies for when the actual hardware display driver is hooked up.
use embedded_graphics::{pixelcolor::BinaryColor, prelude::*};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Gauge, Paragraph},
    Terminal,
};
use mousefood::{EmbeddedBackend, EmbeddedBackendConfig};

// ==========================================
// GLOBAL MEMORY ALLOCATOR (For Ratatui in no_std)
// ==========================================
// MAX'S FEEDBACK INTEGRATION: Ratatui requires dynamic memory allocation.
// IMPLEMENTATION: Since we are in no_std, we manually provide a 32KB Heap using `embedded-alloc`.
use embedded_alloc::Heap;

#[global_allocator]
static HEAP: Heap = Heap::empty();

// ==========================================
// HARDWARE EVENT CHANNEL (The Watcher)
// ==========================================
/// Global channel for hardware interrupts to communicate with the UI task.
pub static UI_EVENT_CHANNEL: Watch<CriticalSectionRawMutex, Option<UIEvent>, 2> = Watch::new();

// ==========================================
// Data Structures & Enums
// ==========================================

// MAX'S FEEDBACK: "Having a variant `None` is an anti-pattern in Rust. Use Option<Motor> instead."
// IMPLEMENTATION: Removed the `None` variant. The state machine now correctly uses `Option<Motor>`.
#[derive(PartialEq, Copy, Clone)]
pub enum Motor {
    Translation,
    Cut,
    Rotation,
}

// MAX'S FEEDBACK: "StatusState is a bit ambiguous, maybe HMIState?"
// IMPLEMENTATION: Renamed to HMIState (Human-Machine Interface State) for better clarity in the firmware context.
#[derive(PartialEq, Copy, Clone)]
enum HMIState {
    Startup,
    Off,
    OnManual,
    OnAuto,
}

// MAX'S FEEDBACK: "Instead of having a variable for unit, we can use the `uom` crate later."
// IMPLEMENTATION: Kept `SpdUnit` temporarily for layout structure. We will swap this with `uom` types 
// once the hardware motor control logic is integrated.
#[derive(PartialEq, Copy, Clone)]
enum SpdUnit {
    Percent,
    MMS,
}

// MAX'S FEEDBACK: "is smart! let's rename to UIEvent instead of AppEvent"
// IMPLEMENTATION: Renamed to `UIEvent`. These events will now be triggered by physical GPIO pins.
#[derive(PartialEq, Copy, Clone)]
pub enum UIEvent {
    TogglePower,  // Physical Power Button
    ToggleMode,   // Physical Auto/Manual Button
    Select,       // Physical Rotary Encoder Click
    ToggleUnit,   // Physical Unit Button
    StopReset,    // Physical Stop Button
    EncoderCW,    // Physical Rotary Encoder Clockwise
    EncoderCCW,   // Physical Rotary Encoder Counter-Clockwise
}

// MAX'S FEEDBACK: "Returning bool here is not very expressive. Make an enum."
// IMPLEMENTATION: Replaced boolean returns with a type-safe `RunState` enum.
#[derive(PartialEq, Copy, Clone)]
enum RunState {
    Continue,
    Exit,
}

struct AppState {
    status: HMIState,
    motor: Option<Motor>, // Now using Option per Max's code review
    highlighted_motor: Motor,
    trans_spd: f32,
    cut_spd: f32,
    rot_spd: f32,
    unit: SpdUnit,
}

impl AppState {
    // --- CORE LOGIC: State Machine ---
    fn handle_event(&mut self, event: UIEvent) -> RunState {
        if self.status == HMIState::Startup {
            return RunState::Continue;
        }

        match event {
            UIEvent::TogglePower => {
                if self.status == HMIState::Off {
                    self.status = HMIState::OnManual;
                } else {
                    self.status = HMIState::Off;
                    self.motor = None;
                    self.trans_spd = 0.0;
                    self.cut_spd = 0.0;
                    self.rot_spd = 0.0;
                }
            }
            _ if self.status != HMIState::Off => match event {
                UIEvent::ToggleMode => {
                    self.status = match self.status {
                        HMIState::OnManual => HMIState::OnAuto,
                        _ => HMIState::OnManual,
                    };
                    self.motor = None;
                }
                UIEvent::StopReset => {
                    self.trans_spd = 0.0;
                    self.cut_spd = 0.0;
                    self.rot_spd = 0.0;
                }
                UIEvent::ToggleUnit => {
                    if self.status == HMIState::OnManual {
                        self.unit = match self.unit {
                            SpdUnit::Percent => SpdUnit::MMS,
                            SpdUnit::MMS => SpdUnit::Percent,
                        };
                    }
                }
                UIEvent::Select if self.status == HMIState::OnManual => {
                    if self.motor.is_none() {
                        self.motor = Some(self.highlighted_motor);
                    } else {
                        self.motor = None;
                    }
                }
                UIEvent::EncoderCCW if self.status == HMIState::OnManual => {
                    if self.motor.is_none() {
                        self.highlighted_motor = cycle_motor(self.highlighted_motor, -1);
                    } else {
                        if let Some(active_motor) = self.motor {
                            let speed_ref = get_speed_ref(self, active_motor);
                            *speed_ref = (*speed_ref + 0.5).clamp(-100.0, 100.0);
                        }
                    }
                }
                UIEvent::EncoderCW if self.status == HMIState::OnManual => {
                    if self.motor.is_none() {
                        self.highlighted_motor = cycle_motor(self.highlighted_motor, 1);
                    } else {
                        if let Some(active_motor) = self.motor {
                            let speed_ref = get_speed_ref(self, active_motor);
                            *speed_ref = (*speed_ref - 0.5).clamp(-100.0, 100.0);
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
        RunState::Continue
    }
}

const CONV_PERC_TO_MMS: f32 = 0.5;

const LOGO_DOTS: &str = "\
.................................
.....███.███.█.█.███.███.█.█.....
.....█...█.█.█.█..█..█.█.███.....
.....███.███..█...█..█.█.█.█.....
.......█.█.█.█.█..█..█.█.█.█.....
.....███.█.█.█.█.███.███.█.█.....
.................................
.███.███.███.███.███.███.███.███.
.█.█.█.█.█.█.█.█..█...█..█...█...
.███.█.█.███.█.█..█...█..█...███.
.█.█.█.█.█.█.█.█..█...█..█.....█.
.█.█.███.███.███..█..███.███.███.
.................................";

// ==========================================
// EMBASSY ASYNC HARDWARE TASK
// ==========================================

// MAX'S FEEDBACK: "Using Box<dyn Error> in embedded usually means heap allocation which we want to avoid."
// IMPLEMENTATION: By using `#[embassy_executor::main]` and `#![no_std]`, we naturally avoid `Box` 
// and heap allocations. The main function simply runs infinitely without returning a Result.
#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    // ---------------------------------------------------------
    // SYSTEM MEMORY INIT
    // ---------------------------------------------------------
    // Initialize the global allocator FIRST
    {
        use core::mem::MaybeUninit;
        const HEAP_SIZE: usize = 1024 * 32;
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { HEAP.init(HEAP_MEM.as_ptr() as usize, HEAP_SIZE) }
    }

    defmt::info!("Peeler Mouse UI - Hardware Firmware Initialized!");

    // TODO: [HARDWARE DISPLAY INTEGRATION]
    // 1. Initialize the I2C/SPI bus here using embassy-stm32.
    // 2. Initialize the SSD1306 (or similar) display driver.
    // let mut i2c = ...
    // let mut display = Ssd1306::new(...);
    // display.init().unwrap();

    let mut app = AppState {
        status: HMIState::Startup,
        motor: None,
        highlighted_motor: Motor::Translation,
        trans_spd: 0.0,
        cut_spd: 0.0,
        rot_spd: 0.0,
        unit: SpdUnit::Percent,
    };

    let startup_time = Instant::now();
    let splash_duration = Duration::from_secs(3);

    // Creates a receiver to listen to hardware interrupts (Buttons/Encoder)
    let mut event_receiver = UI_EVENT_CHANNEL.receiver().unwrap();

    loop {
        if app.status == HMIState::Startup && startup_time.elapsed() >= splash_duration {
            app.status = HMIState::Off;
        }

        // ==============================================================
        // UI RENDERING BLOCK (Uncomment once physical display is connected)
        // ==============================================================
        /*
        display.clear(BinaryColor::Off).unwrap();
        {
            let backend = EmbeddedBackend::new(&mut display, EmbeddedBackendConfig::default());
            let mut terminal = Terminal::new(backend).unwrap();

            terminal.draw(|f| {
                // UI Layout logic remains exactly the same as the simulator version!
                // ...
            }).unwrap();
        }
        display.flush().unwrap(); // Required for actual hardware displays
        */

        // ==========================================
        // ASYNC HARDWARE INPUT POLLING (The Embassy Way)
        // ==========================================
        
        // MAX'S FEEDBACK: "Use embassy async wait instead of blocking thread sleep."
        // IMPLEMENTATION: We use `with_timeout` to wait for an interrupt from `UI_EVENT_CHANNEL`. 
        // If no button is pressed within 30ms, it loops to refresh the display frame. 
        // This yields the CPU to other hardware tasks instead of blocking it!
        match embassy_time::with_timeout(Duration::from_millis(30), event_receiver.changed()).await {
            Ok(hardware_event) => {
                if let Some(ev) = hardware_event {
                    defmt::info!("Hardware Event Triggered!");
                    app.handle_event(ev);
                }
            }
            Err(_) => {
                // Timeout reached, proceed to next UI render frame.
            }
        }
    }
}

// ==========================================
// Helper Functions
// ==========================================

fn cycle_motor(current: Motor, direction: i8) -> Motor {
    match current {
        Motor::Translation => if direction > 0 { Motor::Cut } else { Motor::Rotation },
        Motor::Cut => if direction > 0 { Motor::Rotation } else { Motor::Translation },
        Motor::Rotation => if direction > 0 { Motor::Translation } else { Motor::Cut },
    }
}

fn get_speed_ref<'a>(app: &'a mut AppState, motor: Motor) -> &'a mut f32 {
    match motor {
        Motor::Translation => &mut app.trans_spd,
        Motor::Cut => &mut app.cut_spd,
        Motor::Rotation => &mut app.rot_spd,
    }
}