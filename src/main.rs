#![no_std]
#![no_main]
#![allow(unused_imports)] 
#![allow(dead_code)]      

// ==========================================
// MEMORY ALLOCATION SETUP
// ==========================================
// MAX'S FEEDBACK: "Using Box<dyn Error> in embedded usually means heap allocation which we want to avoid if possible. If required by Ratatui, set up an explicit global allocator."
// IMPLEMENTATION: We explicitly declare the alloc crate and set up `embedded_alloc` to provide the heap memory required by the Ratatui UI framework.
extern crate alloc;
use alloc::format; 

// ==========================================
// EMBASSY & HARDWARE IMPORTS
// ==========================================
use defmt_rtt as _; 
use panic_probe as _; 

use embassy_executor::Spawner;

// MAX'S FEEDBACK: "In the final hardware version, we need to use embassy-time instead of std::time."
// IMPLEMENTATION: Replaced standard library time with embassy_time for hardware-accurate, non-blocking delays.
use embassy_time::{Duration, Instant, Timer};

// MAX'S FEEDBACK: "Within the stm32 firmware you will get notified of button presses and encoder rotations using an embassy::sync::Watch."
// IMPLEMENTATION: Configured a global Watch channel to decouple hardware inputs from UI logic processing.
use embassy_sync::watch::Watch;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use embassy_stm32::gpio::{Input, Pull};

use embedded_graphics::{pixelcolor::BinaryColor, prelude::*};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Gauge, Paragraph},
    Terminal,
};
use mousefood::{EmbeddedBackend, EmbeddedBackendConfig};
use embedded_alloc::Heap;

// ==========================================
// GLOBAL MEMORY ALLOCATOR
// ==========================================
#[global_allocator]
static HEAP: Heap = Heap::empty();

// ==========================================
// HARDWARE EVENT CHANNEL
// ==========================================
pub static UI_EVENT_CHANNEL: Watch<CriticalSectionRawMutex, Option<UIEvent>, 2> = Watch::new();

// ==========================================
// DATA STRUCTURES
// ==========================================

// MAX'S FEEDBACK: "Having a variant `None` inside Motor enum is an anti-pattern in Rust. Use Option<Motor> instead."
// IMPLEMENTATION: Removed `None` variant. The state logic now correctly utilizes `Option<Motor>`.
#[derive(PartialEq, Copy, Clone)]
pub enum Motor {
    Translation,
    Cut,
    Rotation,
}

// MAX'S FEEDBACK: "StatusState is a bit ambiguous, maybe rename to HMIState?"
// IMPLEMENTATION: Renamed properly to reflect Human-Machine Interface states.
#[derive(PartialEq, Copy, Clone)]
enum HMIState {
    Startup,
    Off,
    OnManual,
    OnAuto,
}

#[derive(PartialEq, Copy, Clone)]
enum SpdUnit {
    Percent,
    MMS,
}

// MAX'S FEEDBACK: "Using UI events like this to decouple 'how people input' and 'what the UI should do' is smart! I'd call it UIEvent."
// IMPLEMENTATION: Renamed from AppEvent to UIEvent for clarity.
#[derive(PartialEq, Copy, Clone)]
pub enum UIEvent {
    TogglePower,
    ToggleMode,
    Select,
    ToggleUnit,
    StopReset,
    EncoderCW,
    EncoderCCW,
}

#[derive(PartialEq, Copy, Clone)]
enum RunState {
    Continue,
    Exit,
}

struct AppState {
    status: HMIState,
    motor: Option<Motor>,
    highlighted_motor: Motor,
    trans_spd: f32,
    cut_spd: f32,
    rot_spd: f32,
    unit: SpdUnit,
}

impl AppState {
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
                    } else if let Some(active_motor) = self.motor {
                        let speed_ref = get_speed_ref(self, active_motor);
                        *speed_ref = (*speed_ref + 0.5).clamp(-100.0, 100.0);
                    }
                }
                UIEvent::EncoderCW if self.status == HMIState::OnManual => {
                    if self.motor.is_none() {
                        self.highlighted_motor = cycle_motor(self.highlighted_motor, 1);
                    } else if let Some(active_motor) = self.motor {
                        let speed_ref = get_speed_ref(self, active_motor);
                        *speed_ref = (*speed_ref - 0.5).clamp(-100.0, 100.0);
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
// ASYNC BUTTON TASK (CLEAN & NON-BLOCKING)
// ==========================================
// MAX'S FEEDBACK / EMBASSY UPDATE FIX: "Type-erased EXTI channels (AnyChannel) are deprecated and unsafe."
// IMPLEMENTATION: Removed EXTI complexity. We now use a pure async polling system. 
// Uses generic `Input<'static>` to resolve macro duplication and type mismatch errors.
#[embassy_executor::task(pool_size = 7)]
async fn button_task(input: Input<'static>, event: UIEvent, log_name: &'static str) {
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
            
            // Yield CPU until button is released
            while input.is_low() {
                Timer::after(Duration::from_millis(10)).await;
            }
            Timer::after(Duration::from_millis(100)).await; // Release debounce
        }
    }
}

// ==========================================
// MAIN ASYNC EXECUTOR
// ==========================================

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    {
        use core::mem::MaybeUninit;
        const HEAP_SIZE: usize = 1024 * 64; 
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        
        unsafe { 
            let heap_ptr = core::ptr::addr_of_mut!(HEAP_MEM) as *mut u8 as usize;
            HEAP.init(heap_ptr, HEAP_SIZE);
        }
    }

    defmt::info!("Peeler Mouse UI - Hardware Firmware Initialized!");

    let p = embassy_stm32::init(Default::default());

    // ==========================================
    // SPAWN BUTTON TASKS
    // ==========================================
    // Pass raw typed pins wrapped in Input safely into the generic button task
    spawner.spawn(button_task(Input::new(p.PA0, Pull::Up), UIEvent::TogglePower, "Power Button").unwrap());
    spawner.spawn(button_task(Input::new(p.PA1, Pull::Up), UIEvent::ToggleMode, "Mode Button").unwrap());
    spawner.spawn(button_task(Input::new(p.PA4, Pull::Up), UIEvent::Select, "Encoder Select").unwrap());
    spawner.spawn(button_task(Input::new(p.PB0, Pull::Up), UIEvent::ToggleUnit, "Unit Button").unwrap());
    spawner.spawn(button_task(Input::new(p.PC1, Pull::Up), UIEvent::StopReset, "Stop/Reset").unwrap());
    spawner.spawn(button_task(Input::new(p.PC0, Pull::Up), UIEvent::EncoderCW, "Encoder CW (Right)").unwrap());
    spawner.spawn(button_task(Input::new(p.PA10, Pull::Up), UIEvent::EncoderCCW, "Encoder CCW (Left)").unwrap());

    // ==========================================
    // I2C Setup & Pull-ups
    // ==========================================
    let mut i2c_cfg = embassy_stm32::i2c::Config::default();
    i2c_cfg.sda_pullup = true;
    i2c_cfg.scl_pullup = true;

    let i2c = embassy_stm32::i2c::I2c::new_blocking(
        p.I2C1,
        p.PB8, // SCL
        p.PB9, // SDA
        i2c_cfg,
    );

    // Display Driver Setup
    use ssd1306::{prelude::*, I2CDisplayInterface, Ssd1306};
    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306::new(
        interface,
        DisplaySize128x64, 
        DisplayRotation::Rotate0,
    ).into_buffered_graphics_mode();

    // ==========================================
    // GRACEFUL ERROR HANDLING (FIX FOR BUSWRITEERROR)
    // ==========================================
    // IMPLEMENTATION: Handled display init gracefully to prevent HardFault when display is not connected or incompatible.
    let mut display_ok = false;
    match display.init() {
        Ok(_) => {
            defmt::info!("OLED (SSD1306) Initialized successfully!");
            let _ = display.clear(BinaryColor::Off);
            let _ = display.flush();
            display_ok = true;
        }
        Err(_) => {
            defmt::error!("BusWriteError: OLED init failed! Check SDA/SCL wiring or display model (e.g. MSP1503 uses SSD1327).");
        }
    }

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
    
    let mut event_receiver = UI_EVENT_CHANNEL.receiver().unwrap();

    loop {
        // Direct jump to OnManual after startup splash
        if app.status == HMIState::Startup && startup_time.elapsed() >= splash_duration {
            app.status = HMIState::OnManual; 
        }

        if display_ok {
            let _ = display.clear(BinaryColor::Off);
            
            let backend = EmbeddedBackend::new(&mut display, EmbeddedBackendConfig::default());
            if let Ok(mut terminal) = Terminal::new(backend) {
                let _ = terminal.draw(|f| {
                    let full_area = f.area();

                    let half_screen = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
                        .split(full_area);
                    
                    let inner_area = half_screen[0];

                    if app.status == HMIState::Startup {
                        let splash_logo = Paragraph::new(LOGO_DOTS)
                            .style(Style::default().fg(Color::White))
                            .alignment(Alignment::Center);
                        f.render_widget(splash_logo, inner_area);
                        return;
                    }

                    let main_chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Min(0), Constraint::Length(1)].as_ref())
                        .split(inner_area);

                    let footer = Paragraph::new("SAXION ROBOTICS")
                        .style(Style::default().fg(Color::White))
                        .alignment(Alignment::Center);
                    f.render_widget(footer, main_chunks[1]);

                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(1), Constraint::Length(1), Constraint::Length(1),
                            Constraint::Length(1), Constraint::Length(2), Constraint::Min(0),
                        ].as_ref())
                        .split(main_chunks[0]);

                    let header_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Min(0), Constraint::Length(8)].as_ref())
                        .split(chunks[0]);

                    let (status_text, status_style) = match app.status {
                        HMIState::Off => ("OFF", Style::default().fg(Color::White)),
                        HMIState::OnManual => (" ON/MAN ", Style::default().fg(Color::Black).bg(Color::White).add_modifier(Modifier::BOLD)),
                        HMIState::OnAuto => (" ON/AUTO ", Style::default().fg(Color::Black).bg(Color::White).add_modifier(Modifier::BOLD)),
                        _ => ("", Style::default()),
                    };

                    f.render_widget(Paragraph::new(" System:").style(Style::default().fg(Color::White)), header_chunks[0]);
                    f.render_widget(Paragraph::new(status_text).style(status_style).alignment(Alignment::Right), header_chunks[1]);

                    if app.status == HMIState::Off {
                        let off_p = Paragraph::new("PRESS (A) PWR")
                            .alignment(Alignment::Center)
                            .style(Style::default().fg(Color::White));
                        f.render_widget(off_p, chunks[2]);
                        return;
                    }

                    let render_menu_item = |name: &str, speed: f32, motor_type: Motor| -> Paragraph {
                        let show_speed = if app.unit == SpdUnit::Percent { speed } else { speed * CONV_PERC_TO_MMS };
                        let unit_str = if app.unit == SpdUnit::Percent { "%" } else { "mm/s" };
                        let highlight_mark = if app.motor.is_none() && app.highlighted_motor == motor_type && app.status == HMIState::OnManual { ">" } else { " " };
                        let text = format!("{} {:<7} {:>6.1}{}", highlight_mark, name, show_speed, unit_str);
                        
                        let mut style = Style::default().fg(Color::White);
                        if app.motor == Some(motor_type) {
                            style = style.add_modifier(Modifier::REVERSED).add_modifier(Modifier::BOLD);
                        }
                        Paragraph::new(text).style(style)
                    };

                    f.render_widget(render_menu_item("Trans", app.trans_spd, Motor::Translation), chunks[1]);
                    f.render_widget(render_menu_item("Cut", app.cut_spd, Motor::Cut), chunks[2]);
                    f.render_widget(render_menu_item("Rot", app.rot_spd, Motor::Rotation), chunks[3]);

                    let active_speed = match app.motor {
                        Some(Motor::Translation) => app.trans_spd, 
                        Some(Motor::Cut) => app.cut_spd,
                        Some(Motor::Rotation) => app.rot_spd, 
                        None => 0.0,
                    };

                    if app.motor.is_some() {
                        let show_speed = if app.unit == SpdUnit::Percent { active_speed } else { active_speed * CONV_PERC_TO_MMS };
                        let unit_str = if app.unit == SpdUnit::Percent { "%" } else { "mm/s" };
                        let (gauge_val, arrow) = if active_speed < 0.0 { ((active_speed.abs()) as u16, "<-") } else { (active_speed as u16, "->") };
                        let exact_label = format!("{} {:.1}{}", arrow, show_speed, unit_str);
                        
                        let speed_gauge = Gauge::default()
                            .gauge_style(Style::default().fg(Color::Black).bg(Color::White))
                            .style(Style::default().fg(Color::White).bg(Color::Black))
                            .percent(gauge_val)
                            .label(exact_label);
                        
                        f.render_widget(speed_gauge, chunks[4]);
                    }
                });
            }
            
            let _ = display.flush(); 
        }

        match embassy_time::with_timeout(Duration::from_millis(30), event_receiver.changed()).await {
            Ok(hardware_event) => {
                if let Some(ev) = hardware_event {
                    app.handle_event(ev);
                }
            }
            Err(_) => {}
        }
    }
}

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