// ==========================================
// RATATUI VISUAL INTERFACE
// ==========================================

use alloc::format;
use ratatui::{
    layout::{Alignment, Constraint, Layout}, // Modernized Layout API
    style::{Color, Modifier, Style, Stylize}, // Added Stylize for cleaner syntax
    widgets::{Gauge, Paragraph},
    Frame,
};

use crate::app_state::{AppState, HMIState, Motor, SpdUnit, CONV_PERC_TO_MMS, LOGO_DOTS};

// MAX'S FEEDBACK: Ensure drawing operations are completely decoupled from hardware
// IMPLEMENTATION: This function is pure and only reads from the borrowed AppState.
pub fn draw_ui(f: &mut Frame, app: &AppState) {
    let full_area = f.area();

    // Modern Ratatui Layout API (v0.26+)
    let half_screen = Layout::vertical([
        Constraint::Percentage(50), 
        Constraint::Percentage(50)
    ]).split(full_area);
    
    let inner_area = half_screen[0];

    // --- SPLASH SCREEN RENDER ---
    if app.status == HMIState::Startup {
        let splash_logo = Paragraph::new(LOGO_DOTS)
            .fg(Color::White)
            .alignment(Alignment::Center);
        f.render_widget(splash_logo, inner_area);
        return;
    }

    // --- MAIN UI LAYOUT ---
    let main_chunks = Layout::vertical([
        Constraint::Min(0), 
        Constraint::Length(1)
    ]).split(inner_area);

    let footer = Paragraph::new("SAXION ROBOTICS")
        .fg(Color::White)
        .alignment(Alignment::Center);
    f.render_widget(footer, main_chunks[1]);

    let chunks = Layout::vertical([
        Constraint::Length(1), // Header
        Constraint::Length(1), // Trans
        Constraint::Length(1), // Cut
        Constraint::Length(1), // Rot
        Constraint::Length(2), // Gauge
        Constraint::Min(0),
    ]).split(main_chunks[0]);

    let header_chunks = Layout::horizontal([
        Constraint::Min(0), 
        Constraint::Length(8)
    ]).split(chunks[0]);

    // Format status block
    let (status_text, status_style) = match app.status {
        HMIState::Off => ("OFF", Style::default().fg(Color::White)),
        HMIState::OnManual => (" ON/MAN ", Style::default().fg(Color::Black).bg(Color::White).add_modifier(Modifier::BOLD)),
        HMIState::OnAuto => (" ON/AUTO ", Style::default().fg(Color::Black).bg(Color::White).add_modifier(Modifier::BOLD)),
        _ => ("", Style::default()),
    };

    f.render_widget(Paragraph::new(" System:").fg(Color::White), header_chunks[0]);
    f.render_widget(Paragraph::new(status_text).style(status_style).alignment(Alignment::Right), header_chunks[1]);

    // Handle OFF state visibility
    if app.status == HMIState::Off {
        let off_p = Paragraph::new("PRESS (A) PWR")
            .alignment(Alignment::Center)
            .fg(Color::White);
        f.render_widget(off_p, chunks[2]);
        return;
    }

    // Dynamic menu item renderer
    let render_menu_item = |name: &str, speed: f32, motor_type: Motor| -> Paragraph {
        let show_speed = if app.unit == SpdUnit::Percent { speed } else { speed * CONV_PERC_TO_MMS };
        let unit_str = if app.unit == SpdUnit::Percent { "%" } else { "mm/s" };
        let highlight_mark = if app.motor.is_none() && app.highlighted_motor == motor_type && app.status == HMIState::OnManual { ">" } else { " " };
        let text = format!("{} {:<7} {:>6.1}{}", highlight_mark, name, show_speed, unit_str);
        
        let mut style = Style::default().fg(Color::White);
        if app.motor == Some(motor_type) {
            style = style.add_modifier(Modifier::REVERSED).add_modifier(Modifier::BOLD); // Active selection
        }
        Paragraph::new(text).style(style)
    };

    f.render_widget(render_menu_item("Trans", app.trans_spd, Motor::Translation), chunks[1]);
    f.render_widget(render_menu_item("Cut", app.cut_spd, Motor::Cut), chunks[2]);
    f.render_widget(render_menu_item("Rot", app.rot_spd, Motor::Rotation), chunks[3]);

    // --- ACTIVE MOTOR GAUGE RENDER ---
    let active_speed = match app.motor {
        Some(Motor::Translation) => app.trans_spd, 
        Some(Motor::Cut) => app.cut_spd,
        Some(Motor::Rotation) => app.rot_spd, 
        None => 0.0,
    };

    if app.motor.is_some() {
        let show_speed = if app.unit == SpdUnit::Percent { active_speed } else { active_speed * CONV_PERC_TO_MMS };
        let unit_str = if app.unit == SpdUnit::Percent { "%" } else { "mm/s" };
        
        // CRITICAL FIX: Prevent Ratatui panic by clamping gauge percentage between 0 and 100
        let raw_percent = active_speed.abs() as u16;
        let gauge_val = raw_percent.clamp(0, 100);
        
        let arrow = if active_speed < 0.0 { "<-" } else { "->" };
        let exact_label = format!("{} {:.1}{}", arrow, show_speed, unit_str);
        
        let speed_gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::Black).bg(Color::White))
            .style(Style::default().fg(Color::White).bg(Color::Black))
            .percent(gauge_val)
            .label(exact_label);
        
        f.render_widget(speed_gauge, chunks[4]);
    }
}