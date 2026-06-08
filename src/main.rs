#![no_std]
#![no_main]
#![allow(unused_imports)] 
#![allow(dead_code)]       

// ==========================================
// MEMORY ALLOCATION SETUP
// ==========================================
extern crate alloc;
use embedded_alloc::Heap;

use defmt_rtt as _; 
use panic_probe as _; 

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};

use embassy_stm32::gpio::{Input, Pull};
use embassy_stm32::i2c::{self, I2c};
use embassy_stm32::bind_interrupts;
use embassy_stm32::peripherals;

// Import our decoupled modules
mod app_state;
mod hardware;
mod ui;

use app_state::{AppState, HMIState, UIEvent};
use hardware::{button_task, UI_EVENT_CHANNEL};
use ui::draw_ui;

use embedded_graphics::pixelcolor::BinaryColor;
use ratatui::Terminal;
use mousefood::{EmbeddedBackend, EmbeddedBackendConfig};

// Async OLED driver
use oled_async::{prelude::*, Builder}; 

#[global_allocator]
static HEAP: Heap = Heap::empty();

// ==========================================
// HARDWARE IRQ BINDINGS (STM32G4 Corrected)
// ==========================================
// FIX: In STM32G4 pac, the DMA interrupts are named DMA1_CHANNELx, while the peripherals are DMA1_CHx
bind_interrupts!(struct Irqs {
    I2C1_EV => embassy_stm32::i2c::EventInterruptHandler<peripherals::I2C1>;
    I2C1_ER => embassy_stm32::i2c::ErrorInterruptHandler<peripherals::I2C1>;
    DMA1_CHANNEL1 => embassy_stm32::dma::InterruptHandler<peripherals::DMA1_CH1>;
    DMA1_CHANNEL2 => embassy_stm32::dma::InterruptHandler<peripherals::DMA1_CH2>;
});

// ==========================================
// MAIN ASYNC EXECUTOR
// ==========================================
#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // 1. Initialize Heap for Ratatui
    {
        use core::mem::MaybeUninit;
        const HEAP_SIZE: usize = 1024 * 64; 
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        
        unsafe { 
            let heap_ptr = core::ptr::addr_of_mut!(HEAP_MEM) as *mut u8 as usize;
            HEAP.init(heap_ptr, HEAP_SIZE);
        }
    }

    defmt::info!("Peeler Mouse UI - Async Hardware Firmware Initialized!");
    let p = embassy_stm32::init(Default::default());

    // ==========================================
    // SPAWN BUTTON TASKS
    // ==========================================
    spawner.spawn(button_task(Input::new(p.PA0, Pull::Up), UIEvent::TogglePower, "Power Button").unwrap());
    spawner.spawn(button_task(Input::new(p.PA1, Pull::Up), UIEvent::ToggleMode, "Mode Button").unwrap());
    spawner.spawn(button_task(Input::new(p.PA4, Pull::Up), UIEvent::Select, "Encoder Select").unwrap());
    spawner.spawn(button_task(Input::new(p.PB0, Pull::Up), UIEvent::ToggleUnit, "Unit Button").unwrap());
    spawner.spawn(button_task(Input::new(p.PC1, Pull::Up), UIEvent::StopReset, "Stop/Reset").unwrap());
    spawner.spawn(button_task(Input::new(p.PC0, Pull::Up), UIEvent::EncoderCW, "Encoder CW (Right)").unwrap());
    spawner.spawn(button_task(Input::new(p.PA10, Pull::Up), UIEvent::EncoderCCW, "Encoder CCW (Left)").unwrap());

    // ==========================================
    // ASYNC I2C & DISPLAY SETUP
    // ==========================================
    let i2c_cfg = {
        let mut cfg = embassy_stm32::i2c::Config::default();
        cfg.sda_pullup = true;
        cfg.scl_pullup = true;
        cfg
    };
    
    // FIX: Argument order corrected according to Max's snippet. Irqs must be placed BEFORE dma channels.
    let i2c = I2c::new(
        p.I2C1,
        p.PB8, // SCL
        p.PB9, // SDA
        p.DMA1_CH1, // TX DMA for async transfers
        p.DMA1_CH2, // RX DMA for async transfers
        Irqs,  // <-- IRQs placed here!
        i2c_cfg,
    );

    // FIX: Using the proper external crate for the I2C interface wrapper (Max's method)
    let i2c_interface = display_interface_i2c::I2CInterface::new(i2c, 0x3C, 0x40);

    // FIX: Pass the properly wrapped i2c_interface to connect() to satisfy AsyncWriteOnlyDataCommand trait bounds
    let mut display: GraphicsMode<_, _> = Builder::new(oled_async::displays::ssd1309::Ssd1309_128_64 {})
        .connect(i2c_interface)
        .into();

    let mut display_ok = false;
    
    // Non-blocking Display Initialization
    match display.init().await {
        Ok(_) => {
            defmt::info!("OLED (Async) Initialized successfully!");
            display.clear();
            let _ = display.flush().await; // Non-blocking flush
            display_ok = true;
        }
        Err(_) => {
            defmt::error!("BusWriteError: OLED init failed! Check SDA/SCL wiring.");
        }
    }

    // Initialize logic state
    let mut app = AppState::new();
    let startup_time = Instant::now();
    let splash_duration = Duration::from_secs(3);
    let mut event_receiver = UI_EVENT_CHANNEL.receiver().unwrap();

    // ==========================================
    // MAIN APPLICATION LOOP
    // ==========================================
    loop {
        // Direct jump to OnManual after startup splash
        if app.status == HMIState::Startup && startup_time.elapsed() >= splash_duration {
            app.status = HMIState::OnManual; 
        }

        // Render UI Pipeline
        if display_ok {
            display.clear(); // Sync memory clear
            
            let backend = EmbeddedBackend::new(&mut display, EmbeddedBackendConfig::default());
            if let Ok(mut terminal) = Terminal::new(backend) {
                let _ = terminal.draw(|f| {
                    draw_ui(f, &app); // Delegate UI drawing to ui.rs
                });
            }
            
            let _ = display.flush().await; // Async physical display update
        }

        // Hardware Event Polling via Watch Channel
        match embassy_time::with_timeout(Duration::from_millis(30), event_receiver.changed()).await {
            Ok(hardware_event) => {
                if let Some(ev) = hardware_event {
                    app.handle_event(ev);
                }
            }
            Err(_) => {} // Timeout hit, continue loop
        }
    }
}