// ==========================================
// APP STATE & LOGIC
// ==========================================

use defmt::Format;

pub const CONV_PERC_TO_MMS: f32 = 0.5;

pub const LOGO_DOTS: &str = "\
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

#[derive(PartialEq, Copy, Clone, Format)]
pub enum Motor {
    Translation,
    Cut,
    Rotation,
}

#[derive(PartialEq, Copy, Clone, Format)]
pub enum HMIState {
    Startup,
    Off,
    OnManual,
    OnAuto,
}

#[derive(PartialEq, Copy, Clone, Format)]
pub enum SpdUnit {
    Percent,
    MMS,
}

#[derive(PartialEq, Copy, Clone, Format)]
pub enum UIEvent {
    TogglePower,
    ToggleMode,
    Select,
    ToggleUnit,
    StopReset,
    EncoderCW,
    EncoderCCW,
}

#[derive(PartialEq, Copy, Clone, Format)]
pub enum RunState {
    Continue,
    Exit,
}

#[derive(Clone, Format)]
pub struct AppState {
    pub status: HMIState,
    pub motor: Option<Motor>,
    pub highlighted_motor: Motor,
    pub trans_spd: f32,
    pub cut_spd: f32,
    pub rot_spd: f32,
    pub unit: SpdUnit,
    pub encoder_accum: i8, // Encoder tick accumulator for stepping
}

impl AppState {
    pub fn new() -> Self {
        Self {
            status: HMIState::Startup,
            motor: None,
            highlighted_motor: Motor::Translation,
            trans_spd: 0.0,
            cut_spd: 0.0,
            rot_spd: 0.0,
            unit: SpdUnit::Percent,
            encoder_accum: 0, // Initial accumulator value
        }
    }

    pub fn handle_event(&mut self, event_opt: Option<UIEvent>) -> RunState {
        // Ignore None events (which happen when a button is physically released)
        let event = match event_opt {
            Some(ev) => ev,
            None => return RunState::Continue,
        };

        defmt::info!("[LOGIC] Processing UIEvent: {:?}", event);

        if self.status == HMIState::Startup {
            return RunState::Continue;
        }

        match event {
            UIEvent::TogglePower => {
                self.encoder_accum = 0; // Clear accumulated ticks on state change
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
                    self.encoder_accum = 0; // Clear accumulated ticks
                    self.status = match self.status {
                        HMIState::OnManual => HMIState::OnAuto,
                        _ => HMIState::OnManual,
                    };
                    self.motor = None;
                }
                UIEvent::StopReset => {
                    self.encoder_accum = 0; // Clear accumulated ticks
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
                    self.encoder_accum = 0; // Clear accumulated ticks
                    if self.motor.is_none() {
                        self.motor = Some(self.highlighted_motor);
                    } else {
                        self.motor = None;
                    }
                }
                UIEvent::EncoderCCW if self.status == HMIState::OnManual => {
                    self.encoder_accum -= 1;
                    if self.encoder_accum <= -3 { // Trigger action after 3 CCW ticks
                        self.encoder_accum = 0;
                        if self.motor.is_none() {
                            self.highlighted_motor = cycle_motor(self.highlighted_motor, -1);
                        } else if let Some(active_motor) = self.motor {
                            let speed_ref = get_speed_ref(self, active_motor);
                            *speed_ref = (*speed_ref + 0.5).clamp(-100.0, 100.0);
                        }
                    }
                }
                UIEvent::EncoderCW if self.status == HMIState::OnManual => {
                    self.encoder_accum += 1;
                    if self.encoder_accum >= 3 { // Trigger action after 3 CW ticks
                        self.encoder_accum = 0;
                        if self.motor.is_none() {
                            self.highlighted_motor = cycle_motor(self.highlighted_motor, 1);
                        } else if let Some(active_motor) = self.motor {
                            let speed_ref = get_speed_ref(self, active_motor);
                            *speed_ref = (*speed_ref - 0.5).clamp(-100.0, 100.0);
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
        
        defmt::info!("[LOGIC] New State -> System: {:?}, Motor: {:?}", self.status, self.motor);
        RunState::Continue
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