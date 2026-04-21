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
use std::time::{Duration, Instant};

// MAX'S FEEDBACK: "Using Box<dyn Error> in embedded usually means heap allocation which we want to avoid. Use anyhow."
// FIX: Replaced std::error::Error with anyhow::Result for no-std friendly error handling.
use anyhow::Result; 

use mousefood::{EmbeddedBackend, EmbeddedBackendConfig};

// ==========================================
// Data Structures & Enums
// ==========================================

// MAX'S FEEDBACK: "Having a variant `None` is an anti-pattern in Rust. Use Option<Motor> instead."
// FIX: Removed `None` variant. The state now uses `Option<Motor>`.
#[derive(PartialEq, Copy, Clone)]
enum Motor {
    Translation,
    Cut,
    Rotation,
}

// MAX'S FEEDBACK: "StatusState is a bit ambiguous, maybe HMIState?"
// FIX: Renamed StatusState to HMIState (Human-Machine Interface State).
#[derive(PartialEq, Copy, Clone)]
enum HMIState {
    Startup,
    Off,
    OnManual,
    OnAuto,
}

// MAX'S FEEDBACK: "Instead of having a variable for unit, we can use the `uom` crate later."
// FIX: Kept SpdUnit for the simulator phase, but added a note for hardware integration.
#[derive(PartialEq, Copy, Clone)]
enum SpdUnit {
    Percent,
    MMS,
}

// MAX'S FEEDBACK: "is smart! let's rename to UIEvent instead of AppEvent"
// FIX: Renamed AppEvent to UIEvent.
#[derive(PartialEq, Copy, Clone)]
enum UIEvent {
    TogglePower,  // Button A / Power Button
    ToggleMode,   // Button S / Auto-Manual Button
    Select,       // Button D / Rotary Encoder Click
    ToggleUnit,   // Button F / Unit Button
    StopReset,    // Button G / Stop Button
    EncoderCW,    // Keyboard Down / Rotary Encoder Clockwise
    EncoderCCW,   // Keyboard Up / Rotary Encoder Counter-Clockwise
    Quit,         // Keyboard Q (Simulator only)
}

// MAX'S FEEDBACK: "Returning bool here is not very expressive. Make an enum."
// FIX: Created RunState enum to clearly define if the app should continue or exit.
#[derive(PartialEq, Copy, Clone)]
enum RunState {
    Continue,
    Exit,
}

struct AppState {
    status: HMIState,
    motor: Option<Motor>, // Updated to use Option per Max's feedback
    highlighted_motor: Motor,
    trans_spd: f32,
    cut_spd: f32,
    rot_spd: f32,
    unit: SpdUnit,
}

impl AppState {
    // --- CORE LOGIC: Handles all inputs regardless of source ---
    // Output changed from `bool` to `RunState` per Max's feedback.
    fn handle_event(&mut self, event: UIEvent) -> RunState {
        if event == UIEvent::Quit {
            return RunState::Exit; // Clearly indicates app should close
        }

        if self.status == HMIState::Startup {
            return RunState::Continue;
        }

        match event {
            UIEvent::TogglePower => {
                if self.status == HMIState::Off {
                    self.status = HMIState::OnManual;
                } else {
                    self.status = HMIState::Off;
                    self.motor = None; // Using Option::None
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
// Main Application Loop
// ==========================================

fn main() -> Result<()> {
    let mut display: SimulatorDisplay<BinaryColor> = SimulatorDisplay::new(Size::new(128, 64));
    let output_settings = OutputSettingsBuilder::new().scale(4).pixel_spacing(1).build();
    let mut window = Window::new("🐁 Peeler Mouse - OLED Simulator", &output_settings);

    let mut app = AppState {
        status: HMIState::Startup,
        motor: None, // Start with no active motor
        highlighted_motor: Motor::Translation,
        trans_spd: 0.0,
        cut_spd: 0.0,
        rot_spd: 0.0,
        unit: SpdUnit::Percent,
    };

    let startup_time = Instant::now();
    let splash_duration = Duration::from_secs(3);

    'running: loop {
        if app.status == HMIState::Startup && startup_time.elapsed() >= splash_duration {
            app.status = HMIState::Off;
        }

        display.clear(BinaryColor::Off)?;
        {
            let backend = EmbeddedBackend::new(&mut display, EmbeddedBackendConfig::default());
            let mut terminal = Terminal::new(backend).unwrap();

            terminal.draw(|f| {
                let inner_area = f.area();

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
            }).unwrap();
        }
        window.update(&display);

        for event in window.events() {
            if let SimulatorEvent::KeyDown { keycode, .. } = event {
                let ui_event = match keycode {
                    Keycode::Q => Some(UIEvent::Quit),
                    Keycode::A => Some(UIEvent::TogglePower),
                    Keycode::S => Some(UIEvent::ToggleMode),
                    Keycode::D => Some(UIEvent::Select),
                    Keycode::F => Some(UIEvent::ToggleUnit),
                    Keycode::G => Some(UIEvent::StopReset),
                    Keycode::Up => Some(UIEvent::EncoderCCW),
                    Keycode::Down => Some(UIEvent::EncoderCW),
                    _ => None,
                };

                if let Some(e) = ui_event {
                    if app.handle_event(e) == RunState::Exit {
                        break 'running;
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    Ok(())
}

// ==========================================
// Helper Functions Architecture
// ==========================================
//
// MAX'S FEEDBACK INTEGRATION:
// Previously, these functions relied on `Motor::None` to represent an inactive state.
// Max pointed out that having `None` as a variant inside an enum is a Rust anti-pattern.
// FIX: We refactored `Motor` to only contain physical motors. The active state is now 
// handled via `Option<Motor>` in the AppState.
//
// HOW THEY CONNECT:
// 1. `cycle_motor`: Handles the UI navigation. When `Option<Motor>` is `None` (user is browsing),
//    turning the encoder calls this function to cycle the `highlighted_motor` up or down.
// 2. `get_speed_ref`: Handles data mutation. When `Option<Motor>` is `Some(motor)` (user selected a motor),
//    turning the encoder calls this function to get a direct mutable reference (`&mut f32`) to the target speed.

/// Cycles through the available physical motors for UI menu navigation.
/// Direction: 1 for clockwise (down), -1 for counter-clockwise (up).
fn cycle_motor(current: Motor, direction: i8) -> Motor {
    match current {
        Motor::Translation => if direction > 0 { Motor::Cut } else { Motor::Rotation },
        Motor::Cut => if direction > 0 { Motor::Rotation } else { Motor::Translation },
        Motor::Rotation => if direction > 0 { Motor::Translation } else { Motor::Cut },
    }
}

/// Returns a mutable reference to the specific motor's speed variable in the AppState.
/// This allows direct modification of the speed value via pointer-like referencing.
fn get_speed_ref<'a>(app: &'a mut AppState, motor: Motor) -> &'a mut f32 {
    match motor {
        Motor::Translation => &mut app.trans_spd,
        Motor::Cut => &mut app.cut_spd,
        Motor::Rotation => &mut app.rot_spd,
    }
}