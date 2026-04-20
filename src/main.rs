use embedded_graphics::{pixelcolor::BinaryColor, prelude::*};
use embedded_graphics_simulator::{
    sdl2::Keycode, OutputSettingsBuilder, SimulatorDisplay, SimulatorEvent, Window,
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Gauge, Paragraph},
    Terminal,
};
use std::{
    error::Error,
    time::{Duration, Instant},
};

use mousefood::{EmbeddedBackend, EmbeddedBackendConfig};

// ==========================================
// Data Structures & Enums
// ==========================================

#[derive(PartialEq, Copy, Clone)]
enum Motor {
    None,
    Translation,
    Cut,
    Rotation,
}

#[derive(PartialEq, Copy, Clone)]
enum StatusState {
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

// --- NEW: Unified Input Events ---
// This decouples the UI from the hardware/keyboard.
#[derive(PartialEq, Copy, Clone)]
enum AppEvent {
    TogglePower,  // Button A / Power Button
    ToggleMode,   // Button S / Auto-Manual Button
    Select,       // Button D / Rotary Encoder Click
    ToggleUnit,   // Button F / Unit Button
    StopReset,    // Button G / Stop Button
    EncoderCW,    // Keyboard Down / Rotary Encoder Clockwise
    EncoderCCW,   // Keyboard Up / Rotary Encoder Counter-Clockwise
    Quit,         // Keyboard Q (Simulator only)
}

struct AppState {
    status: StatusState,
    motor: Motor,
    highlighted_motor: Motor,
    trans_spd: f32,
    cut_spd: f32,
    rot_spd: f32,
    unit: SpdUnit,
}

impl AppState {
    // --- CORE LOGIC: Handles all inputs regardless of source ---
    fn handle_event(&mut self, event: AppEvent) -> bool {
        if event == AppEvent::Quit {
            return false; // Signal to exit application
        }

        // Ignore inputs during startup splash screen
        if self.status == StatusState::Startup {
            return true;
        }

        match event {
            AppEvent::TogglePower => {
                if self.status == StatusState::Off {
                    self.status = StatusState::OnManual;
                } else {
                    self.status = StatusState::Off;
                    self.motor = Motor::None;
                    self.trans_spd = 0.0;
                    self.cut_spd = 0.0;
                    self.rot_spd = 0.0;
                }
            }
            _ if self.status != StatusState::Off => match event {
                AppEvent::ToggleMode => {
                    self.status = match self.status {
                        StatusState::OnManual => StatusState::OnAuto,
                        _ => StatusState::OnManual,
                    };
                    self.motor = Motor::None;
                }
                AppEvent::StopReset => {
                    self.trans_spd = 0.0;
                    self.cut_spd = 0.0;
                    self.rot_spd = 0.0;
                }
                AppEvent::ToggleUnit => {
                    if self.status == StatusState::OnManual {
                        self.unit = match self.unit {
                            SpdUnit::Percent => SpdUnit::MMS,
                            SpdUnit::MMS => SpdUnit::Percent,
                        };
                    }
                }
                AppEvent::Select if self.status == StatusState::OnManual => {
                    if self.motor == Motor::None {
                        self.motor = self.highlighted_motor;
                    } else {
                        self.motor = Motor::None;
                    }
                }
                AppEvent::EncoderCCW if self.status == StatusState::OnManual => {
                    if self.motor == Motor::None {
                        self.highlighted_motor = cycle_motor(self.highlighted_motor, -1);
                    } else {
                        let speed_ref = get_speed_ref(self);
                        *speed_ref = (*speed_ref + 0.5).clamp(-100.0, 100.0);
                    }
                }
                AppEvent::EncoderCW if self.status == StatusState::OnManual => {
                    if self.motor == Motor::None {
                        self.highlighted_motor = cycle_motor(self.highlighted_motor, 1);
                    } else {
                        let speed_ref = get_speed_ref(self);
                        *speed_ref = (*speed_ref - 0.5).clamp(-100.0, 100.0);
                    }
                }
                _ => {}
            },
            _ => {}
        }
        true // Continue running
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
// Main Application Loop
// ==========================================

fn main() -> Result<(), Box<dyn Error>> {
    let mut display: SimulatorDisplay<BinaryColor> = SimulatorDisplay::new(Size::new(128, 64));
    let output_settings = OutputSettingsBuilder::new().scale(4).pixel_spacing(1).build();
    let mut window = Window::new("🐁 Peeler Mouse - OLED Simulator", &output_settings);

    let mut app = AppState {
        status: StatusState::Startup,
        motor: Motor::None,
        highlighted_motor: Motor::Translation,
        trans_spd: 0.0,
        cut_spd: 0.0,
        rot_spd: 0.0,
        unit: SpdUnit::Percent,
    };

    let startup_time = Instant::now();
    let splash_duration = Duration::from_secs(3);

    // Hardware simulation variables
    // let mut last_encoder_count = 0; 

    'running: loop {
        if app.status == StatusState::Startup && startup_time.elapsed() >= splash_duration {
            app.status = StatusState::Off;
        }

        // --- 1. RENDER UI ---
        display.clear(BinaryColor::Off)?;
        {
            let backend = EmbeddedBackend::new(&mut display, EmbeddedBackendConfig::default());
            let mut terminal = Terminal::new(backend).unwrap();

            terminal.draw(|f| {
                let inner_area = f.area();

                if app.status == StatusState::Startup {
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
                    StatusState::Off => ("OFF", Style::default().fg(Color::White)),
                    StatusState::OnManual => (" ON/MAN ", Style::default().fg(Color::Black).bg(Color::White).add_modifier(Modifier::BOLD)),
                    StatusState::OnAuto => (" ON/AUTO ", Style::default().fg(Color::Black).bg(Color::White).add_modifier(Modifier::BOLD)),
                    _ => ("", Style::default()),
                };

                f.render_widget(Paragraph::new(" System:").style(Style::default().fg(Color::White)), header_chunks[0]);
                f.render_widget(Paragraph::new(status_text).style(status_style).alignment(Alignment::Right), header_chunks[1]);

                if app.status == StatusState::Off {
                    let off_p = Paragraph::new("PRESS (A) PWR")
                        .alignment(Alignment::Center)
                        .style(Style::default().fg(Color::White));
                    f.render_widget(off_p, chunks[2]);
                    return;
                }

                let render_menu_item = |name: &str, speed: f32, motor_type: Motor| -> Paragraph {
                    let show_speed = if app.unit == SpdUnit::Percent { speed } else { speed * CONV_PERC_TO_MMS };
                    let unit_str = if app.unit == SpdUnit::Percent { "%" } else { "mm/s" };
                    let highlight_mark = if app.motor == Motor::None && app.highlighted_motor == motor_type && app.status == StatusState::OnManual { ">" } else { " " };
                    let text = format!("{} {:<7} {:>6.1}{}", highlight_mark, name, show_speed, unit_str);
                    
                    let mut style = Style::default().fg(Color::White);
                    if app.motor == motor_type {
                        style = style.add_modifier(Modifier::REVERSED).add_modifier(Modifier::BOLD);
                    }
                    Paragraph::new(text).style(style)
                };

                f.render_widget(render_menu_item("Trans", app.trans_spd, Motor::Translation), chunks[1]);
                f.render_widget(render_menu_item("Cut", app.cut_spd, Motor::Cut), chunks[2]);
                f.render_widget(render_menu_item("Rot", app.rot_spd, Motor::Rotation), chunks[3]);

                let active_speed = match app.motor {
                    Motor::Translation => app.trans_spd, Motor::Cut => app.cut_spd,
                    Motor::Rotation => app.rot_spd, Motor::None => 0.0,
                };

                if app.motor != Motor::None {
                    let show_speed = if app.unit == SpdUnit::Percent { active_speed } else { active_speed * CONV_PERC_TO_MMS };
                    let unit_str = if app.unit == SpdUnit::Percent { "%" } else { "mm/s" };
                    let (gauge_val, arrow) = if active_speed < 0.0 { ((active_speed.abs()) as u16, "<-") } else { (active_speed as u16, "->") };
                    let exact_label = format!("{} {:.1}{}", arrow, show_speed, unit_str);
                    
                    let speed_gauge = Gauge::default()
                        .gauge_style(Style::default().fg(Color::White).bg(Color::Black))
                        .style(Style::default().fg(Color::White).bg(Color::Black))
                        .percent(gauge_val)
                        .label(exact_label);
                    
                    f.render_widget(speed_gauge, chunks[4]);
                }
            }).unwrap();
        }
        window.update(&display);

        // ===================================================================
        // --- 2. INPUT HANDLING (CHOOSE EITHER SIMULATOR OR HARDWARE) ---
        // ===================================================================

        // -------------------------------------------------------------------
        // OPTION A: SIMULATOR KEYBOARD INPUT (Active for PC Testing)
        // -------------------------------------------------------------------
        for event in window.events() {
            if let SimulatorEvent::KeyDown { keycode, .. } = event {
                let app_event = match keycode {
                    Keycode::Q => Some(AppEvent::Quit),
                    Keycode::A => Some(AppEvent::TogglePower),
                    Keycode::S => Some(AppEvent::ToggleMode),
                    Keycode::D => Some(AppEvent::Select),
                    Keycode::F => Some(AppEvent::ToggleUnit),
                    Keycode::G => Some(AppEvent::StopReset),
                    Keycode::Up => Some(AppEvent::EncoderCCW),
                    Keycode::Down => Some(AppEvent::EncoderCW),
                    _ => None,
                };

                if let Some(e) = app_event {
                    if !app.handle_event(e) {
                        break 'running;
                    }
                }
            }
        }

        // -------------------------------------------------------------------
        // OPTION B: HARDWARE GPIO INPUT (Commented out for actual MCU)
        // -------------------------------------------------------------------
        /*
        // 1. Read Rotary Encoder
        // let current_encoder_count = hardware_encoder.get_count();
        // if current_encoder_count > last_encoder_count {
        //     app.handle_event(AppEvent::EncoderCW);
        // } else if current_encoder_count < last_encoder_count {
        //     app.handle_event(AppEvent::EncoderCCW);
        // }
        // last_encoder_count = current_encoder_count;

        // 2. Read Physical Buttons (Debounced)
        // if hardware_knob_btn.is_pressed() { app.handle_event(AppEvent::Select); }
        // if hardware_power_btn.is_pressed() { app.handle_event(AppEvent::TogglePower); }
        // if hardware_mode_btn.is_pressed() { app.handle_event(AppEvent::ToggleMode); }
        // if hardware_unit_btn.is_pressed() { app.handle_event(AppEvent::ToggleUnit); }
        // if hardware_stop_btn.is_pressed() { app.handle_event(AppEvent::StopReset); }
        */

        std::thread::sleep(Duration::from_millis(30));
    }

    Ok(())
}

// ==========================================
// Helper Functions
// ==========================================

fn cycle_motor(current: Motor, direction: i8) -> Motor {
    match current {
        Motor::Translation => if direction > 0 { Motor::Cut } else { Motor::Rotation },
        Motor::Cut => if direction > 0 { Motor::Rotation } else { Motor::Translation },
        Motor::Rotation => if direction > 0 { Motor::Translation } else { Motor::Cut },
        Motor::None => Motor::Translation,
    }
}

fn get_speed_ref(app: &mut AppState) -> &mut f32 {
    match app.motor {
        Motor::Translation => &mut app.trans_spd,
        Motor::Cut => &mut app.cut_spd,
        Motor::Rotation => &mut app.rot_spd,
        Motor::None => &mut app.trans_spd,
    }
}