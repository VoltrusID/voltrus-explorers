//! A simulated machine on the virtual bus: a diesel genset with an engine ECU (J1939, 29-bit),
//! a generator controller and a cooling pump drive (11-bit, described by [`SIM_DBC`]), a CANopen
//! I/O node with a heartbeat, and OBD-II answers on 0x7E8. Values move with a throttle the user sets.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::bus::{BusEvent, CanHandle, CanSender, VirtualBus};
use crate::dbc::{Dbc, encode_message};
use crate::frame::CanFrame;

/// The simulator's DBC: load it in the app to decode what the simulator sends.
pub const SIM_DBC: &str = r#"VERSION "Voltrus CAN Explorer simulator 1"

NS_ :
	CM_
	BA_DEF_
	BA_
	VAL_

BS_:

BU_: ECU GEN PUMP IO BMS Tester

BO_ 2364539904 EEC1: 8 ECU
 SG_ EngTorqueMode : 0|4@1+ (1,0) [0|15] "" Tester
 SG_ DriverDemandTorque : 8|8@1+ (1,-125) [-125|125] "%" Tester
 SG_ ActualTorque : 16|8@1+ (1,-125) [-125|125] "%" Tester
 SG_ EngineSpeed : 24|16@1+ (0.125,0) [0|8031.875] "rpm" Tester

BO_ 2566843904 ET1: 8 ECU
 SG_ CoolantTemp : 0|8@1+ (1,-40) [-40|210] "degC" Tester
 SG_ FuelTemp : 8|8@1+ (1,-40) [-40|210] "degC" Tester
 SG_ OilTemp : 16|16@1+ (0.03125,-273) [-273|1734.96875] "degC" Tester

BO_ 2566844160 EFLP1: 8 ECU
 SG_ OilPressure : 24|8@1+ (4,0) [0|1000] "kPa" Tester

BO_ 2566841600 HOURS: 8 ECU
 SG_ EngineHours : 0|32@1+ (0.05,0) [0|210554060.75] "h" Tester

BO_ 2566834688 DM1: 8 ECU
 SG_ MalfunctionLamp : 6|2@1+ (1,0) [0|3] "" Tester
 SG_ AmberLamp : 10|2@1+ (1,0) [0|3] "" Tester
 SG_ SPN : 16|19@1+ (1,0) [0|524287] "" Tester
 SG_ FMI : 35|5@1+ (1,0) [0|31] "" Tester
 SG_ OccurrenceCount : 48|7@1+ (1,0) [0|126] "" Tester

BO_ 801 GenStatus: 8 GEN
 SG_ Voltage : 7|16@0+ (0.1,0) [0|6553.5] "V" Tester
 SG_ Frequency : 23|16@0+ (0.01,0) [0|655.35] "Hz" Tester
 SG_ Current : 39|16@0+ (0.1,0) [0|6553.5] "A" Tester
 SG_ Breaker : 48|2@1+ (1,0) [0|3] "" Tester
 SG_ GenState : 52|4@1+ (1,0) [0|15] "" Tester

BO_ 802 GenPower: 8 GEN
 SG_ ActivePower : 0|32@1- (0.1,0) [-214748364.8|214748364.7] "kW" Tester
 SG_ PowerFactor : 32|16@1- (0.001,0) [-1|1] "" Tester

BO_ 1040 PumpDrive: 8 PUMP
 SG_ Mode M : 0|8@1+ (1,0) [0|255] "" Tester
 SG_ Speed m0 : 8|16@1+ (1,0) [0|65535] "rpm" Tester
 SG_ Flow m0 : 24|16@1+ (0.1,0) [0|6553.5] "l/min" Tester
 SG_ MotorTemp m1 : 8|8@1+ (1,-40) [-40|215] "degC" Tester
 SG_ FaultCode m1 : 16|16@1+ (1,0) [0|65535] "" Tester

BO_ 1296 PumpCommand: 8 Tester
 SG_ SpeedSetpoint : 0|16@1+ (1,0) [0|3000] "rpm" PUMP
 SG_ Enable : 16|1@1+ (1,0) [0|1] "" PUMP

BO_ 1297 ThrottleCommand: 8 Tester
 SG_ Throttle : 0|8@1+ (0.5,0) [0|100] "%" ECU

CM_ BO_ 2364539904 "J1939 PGN 61444 Electronic Engine Controller 1, from the engine (SA 00).";
CM_ BO_ 2566843904 "J1939 PGN 65262 Engine Temperature 1.";
CM_ BO_ 2566844160 "J1939 PGN 65263 Engine Fluid Level/Pressure 1.";
CM_ BO_ 2566841600 "J1939 PGN 65253 Engine Hours.";
CM_ BO_ 2566834688 "J1939 PGN 65226 DM1, single-frame form with one trouble code.";
CM_ BO_ 1184 "Battery pack (CAN FD, 64 bytes, bit rate switch). CellPage selects which 16 of the 64 cell voltages the frame carries.";
CM_ BO_ 1040 "Cooling pump drive. Mode 0: speed and flow, mode 1: motor temperature and fault code.";
CM_ BO_ 1296 "Send this to the simulator to set the pump speed.";
BO_ 1184 BatteryPackFD: 64 BMS
 SG_ CellPage M : 0|8@1+ (1,0) [0|3] "" Tester
 SG_ PackVoltage : 8|16@1+ (0.01,0) [0|655.35] "V" Tester
 SG_ PackCurrent : 24|16@1- (0.1,0) [-3276.8|3276.7] "A" Tester
 SG_ SOC : 40|8@1+ (0.5,0) [0|100] "%" Tester
 SG_ SOH : 48|8@1+ (0.5,0) [0|100] "%" Tester
 SG_ TempMin : 56|8@1+ (1,-40) [-40|215] "degC" Tester
 SG_ TempMax : 64|8@1+ (1,-40) [-40|215] "degC" Tester
 SG_ CellVoltMin : 72|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ CellVoltMax : 88|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ PackState : 104|4@1+ (1,0) [0|15] "" Tester
 SG_ BalancingActive : 108|1@1+ (1,0) [0|1] "" Tester
 SG_ ContactorClosed : 109|1@1+ (1,0) [0|1] "" Tester
 SG_ CellMinIndex : 112|8@1+ (1,0) [1|64] "" Tester
 SG_ CellMaxIndex : 120|8@1+ (1,0) [1|64] "" Tester
 SG_ InsulationRes : 128|16@1+ (1,0) [0|65535] "kOhm" Tester
 SG_ Cell01 m0 : 192|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell02 m0 : 208|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell03 m0 : 224|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell04 m0 : 240|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell05 m0 : 256|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell06 m0 : 272|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell07 m0 : 288|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell08 m0 : 304|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell09 m0 : 320|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell10 m0 : 336|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell11 m0 : 352|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell12 m0 : 368|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell13 m0 : 384|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell14 m0 : 400|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell15 m0 : 416|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell16 m0 : 432|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell17 m1 : 192|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell18 m1 : 208|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell19 m1 : 224|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell20 m1 : 240|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell21 m1 : 256|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell22 m1 : 272|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell23 m1 : 288|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell24 m1 : 304|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell25 m1 : 320|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell26 m1 : 336|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell27 m1 : 352|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell28 m1 : 368|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell29 m1 : 384|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell30 m1 : 400|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell31 m1 : 416|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell32 m1 : 432|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell33 m2 : 192|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell34 m2 : 208|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell35 m2 : 224|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell36 m2 : 240|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell37 m2 : 256|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell38 m2 : 272|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell39 m2 : 288|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell40 m2 : 304|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell41 m2 : 320|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell42 m2 : 336|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell43 m2 : 352|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell44 m2 : 368|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell45 m2 : 384|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell46 m2 : 400|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell47 m2 : 416|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell48 m2 : 432|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell49 m3 : 192|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell50 m3 : 208|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell51 m3 : 224|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell52 m3 : 240|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell53 m3 : 256|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell54 m3 : 272|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell55 m3 : 288|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell56 m3 : 304|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell57 m3 : 320|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell58 m3 : 336|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell59 m3 : 352|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell60 m3 : 368|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell61 m3 : 384|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell62 m3 : 400|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell63 m3 : 416|16@1+ (0.001,0) [0|65.535] "V" Tester
 SG_ Cell64 m3 : 432|16@1+ (0.001,0) [0|65.535] "V" Tester

CM_ BO_ 1297 "Send this to the simulator to set the engine throttle.";
CM_ SG_ 2364539904 EngineSpeed "Actual engine speed.";
CM_ SG_ 801 Breaker "Generator breaker position.";
BA_DEF_ BO_ "GenMsgCycleTime" INT 0 65535;
BA_DEF_ BO_ "VFrameFormat" ENUM "StandardCAN","ExtendedCAN","reserved","reserved","reserved","reserved","reserved","reserved","reserved","reserved","reserved","reserved","reserved","reserved","StandardCAN_FD","ExtendedCAN_FD";
BA_DEF_ BO_ "CANFD_BRS" ENUM "0","1";
BA_DEF_DEF_ "VFrameFormat" "StandardCAN";
BA_DEF_DEF_ "CANFD_BRS" "0";
BA_ "GenMsgCycleTime" BO_ 2364539904 20;
BA_ "GenMsgCycleTime" BO_ 2566843904 1000;
BA_ "GenMsgCycleTime" BO_ 2566844160 500;
BA_ "GenMsgCycleTime" BO_ 2566841600 1000;
BA_ "GenMsgCycleTime" BO_ 2566834688 1000;
BA_ "GenMsgCycleTime" BO_ 801 100;
BA_ "GenMsgCycleTime" BO_ 802 100;
BA_ "GenMsgCycleTime" BO_ 1040 50;
BA_ "GenMsgCycleTime" BO_ 1184 100;
BA_ "VFrameFormat" BO_ 1184 14;
BA_ "CANFD_BRS" BO_ 1184 1;
VAL_ 2364539904 EngTorqueMode 0 "Low idle governor" 1 "Accelerator pedal" 2 "Cruise control" 3 "PTO governor" 15 "Not available" ;
VAL_ 801 Breaker 0 "Open" 1 "Closed" 2 "Tripped" 3 "Not available" ;
VAL_ 801 GenState 0 "Stopped" 1 "Starting" 2 "Warm-up" 3 "Running" 4 "Cool-down" 5 "Fault" ;
VAL_ 1040 Mode 0 "Process data" 1 "Diagnostics" ;
VAL_ 1184 PackState 0 "Standby" 1 "Charging" 2 "Discharging" 3 "Fault" ;
"#;

/// CANopen node id of the simulated I/O module.
pub const CANOPEN_NODE: u8 = 0x11;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimState {
    pub running: bool,
    /// 0 to 100 %.
    pub throttle: f64,
    pub breaker_closed: bool,
    pub pump_enabled: bool,
    pub pump_setpoint: f64,
    /// Raise an active trouble code (SPN 110 coolant temperature, FMI 0) in DM1.
    pub fault: bool,
    /// Multiplies every message rate; 1.0 is the DBC cycle times.
    pub rate: f64,
    /// Frames sent since start.
    pub sent: u64,
    // Live values, for the dashboard.
    pub rpm: f64,
    pub coolant: f64,
    pub oil_pressure: f64,
    pub load_kw: f64,
    pub frequency: f64,
    pub pump_speed: f64,
    pub hours: f64,
    /// Send the CAN FD battery pack message (BatteryPackFD, 0x4A0).
    pub fd_battery: bool,
    /// Battery state of charge, %.
    pub soc: f64,
    /// Battery current, A: positive charging.
    pub pack_current: f64,
    pub pack_voltage: f64,
    pub pack_temp: f64,
}

impl Default for SimState {
    fn default() -> Self {
        SimState {
            running: true,
            throttle: 30.0,
            breaker_closed: true,
            pump_enabled: true,
            pump_setpoint: 1450.0,
            fault: false,
            rate: 1.0,
            sent: 0,
            rpm: 0.0,
            coolant: 25.0,
            oil_pressure: 0.0,
            load_kw: 0.0,
            frequency: 0.0,
            pump_speed: 0.0,
            hours: 1234.5,
            fd_battery: true,
            soc: 62.0,
            pack_current: 0.0,
            pack_voltage: 0.0,
            pack_temp: 24.0,
        }
    }
}

/// Changes the user can make from the dashboard.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SimAction {
    SetRunning { running: bool },
    SetThrottle { value: f64 },
    SetBreaker { closed: bool },
    SetPump { enabled: bool, setpoint: f64 },
    SetFault { active: bool },
    SetRate { rate: f64 },
    SetFdBattery { enabled: bool },
}

pub struct Simulator {
    state: Arc<Mutex<SimState>>,
    task: JoinHandle<()>,
    handle: Option<CanHandle>,
}

impl Simulator {
    /// Attaches the simulator to `bus` and starts sending.
    pub fn start(bus: &VirtualBus) -> Simulator {
        let mut handle = bus.attach();
        let events = handle.take_events().expect("fresh handle");
        let state = Arc::new(Mutex::new(SimState::default()));
        let task = tokio::spawn(run(handle.sender(), events, state.clone()));
        Simulator { state, task, handle: Some(handle) }
    }

    pub async fn state(&self) -> SimState {
        self.state.lock().await.clone()
    }

    pub async fn apply(&self, action: SimAction) {
        let mut s = self.state.lock().await;
        match action {
            SimAction::SetRunning { running } => s.running = running,
            SimAction::SetThrottle { value } => s.throttle = value.clamp(0.0, 100.0),
            SimAction::SetBreaker { closed } => s.breaker_closed = closed,
            SimAction::SetPump { enabled, setpoint } => {
                s.pump_enabled = enabled;
                s.pump_setpoint = setpoint.clamp(0.0, 3000.0);
            }
            SimAction::SetFault { active } => s.fault = active,
            SimAction::SetRate { rate } => s.rate = rate.clamp(0.1, 10.0),
            SimAction::SetFdBattery { enabled } => s.fd_battery = enabled,
        }
    }

    pub async fn stop(mut self) {
        self.task.abort();
        if let Some(h) = self.handle.take() {
            h.close().await;
        }
    }
}

impl Drop for Simulator {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Which page the multiplexed messages send next.
#[derive(Default)]
struct Pages {
    pump: u8,
    cells: usize,
}

struct Schedule {
    name: String,
    period_ms: u64,
    next_ms: u64,
}

async fn run(tx: CanSender, mut events: tokio::sync::mpsc::UnboundedReceiver<BusEvent>, state: Arc<Mutex<SimState>>) {
    let dbc = Dbc::parse(SIM_DBC).expect("built-in DBC parses");
    let mut sched: Vec<Schedule> = dbc
        .messages
        .iter()
        .filter_map(|m| m.cycle_ms.map(|c| (m.name.as_str(), c)))
        .map(|(n, c)| Schedule { name: n.to_string(), period_ms: c as u64, next_ms: 0 })
        .collect();
    sched.push(Schedule { name: "heartbeat".into(), period_ms: 1000, next_ms: 0 });
    sched.push(Schedule { name: "tpdo1".into(), period_ms: 200, next_ms: 0 });

    let tick = Duration::from_millis(5);
    let mut clock_ms: u64 = 0;
    let mut pages = Pages::default();
    let mut timer = tokio::time::interval(tick);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Boot-up message of the CANopen node.
    let _ = tx.send(CanFrame::new(0x700 + CANOPEN_NODE as u32, &[0]).unwrap()).await;

    loop {
        tokio::select! {
            ev = events.recv() => match ev {
                Some(BusEvent::Frame(f)) => {
                    if let Some(reply) = handle_rx(&dbc, &f.frame, &state).await {
                        let _ = tx.send(reply).await;
                    }
                }
                Some(BusEvent::Closed(_)) | None => break,
                Some(_) => {}
            },
            _ = timer.tick() => {
                clock_ms += tick.as_millis() as u64;
                let mut out = Vec::new();
                {
                    let mut s = state.lock().await;
                    physics(&mut s, tick.as_secs_f64());
                    let rate = s.rate;
                    for e in sched.iter_mut() {
                        if clock_ms >= e.next_ms {
                            e.next_ms = clock_ms + ((e.period_ms as f64 / rate).max(1.0) as u64);
                            if let Some(f) = build(&dbc, &e.name, &s, &mut pages) {
                                out.push(f);
                            }
                        }
                    }
                    s.sent += out.len() as u64;
                }
                for f in out {
                    if tx.send(f).await.is_err() {
                        return;
                    }
                }
            }
        }
    }
}

fn approach(v: &mut f64, target: f64, rate_per_s: f64, dt: f64) {
    let k = (rate_per_s * dt).min(1.0);
    *v += (target - *v) * k;
}

fn physics(s: &mut SimState, dt: f64) {
    let target_rpm = if s.running { 800.0 + s.throttle * 10.0 } else { 0.0 };
    approach(&mut s.rpm, target_rpm, 2.0, dt);
    let target_coolant = if s.running { 78.0 + s.throttle * 0.12 + if s.fault { 30.0 } else { 0.0 } } else { 25.0 };
    approach(&mut s.coolant, target_coolant, 0.05, dt);
    let target_oil = if s.running { 180.0 + s.rpm * 0.15 } else { 0.0 };
    approach(&mut s.oil_pressure, target_oil, 3.0, dt);
    s.frequency = s.rpm / 30.0;
    let target_kw = if s.running && s.breaker_closed { 20.0 + s.throttle * 2.4 } else { 0.0 };
    approach(&mut s.load_kw, target_kw, 1.0, dt);
    let target_pump = if s.pump_enabled { s.pump_setpoint } else { 0.0 };
    approach(&mut s.pump_speed, target_pump, 1.5, dt);
    if s.running {
        s.hours += dt / 3600.0;
    }
    // Battery: charged from the generator while it runs on load, discharged otherwise. 100 Ah, 64 NMC cells.
    let target_a = if s.running && s.breaker_closed { 12.0 + s.throttle * 0.4 } else { -18.0 };
    approach(&mut s.pack_current, target_a, 1.0, dt);
    // Sped up 60x so the state of charge visibly moves.
    s.soc = (s.soc + s.pack_current * dt * 60.0 / 3600.0).clamp(5.0, 98.0);
    if s.soc >= 98.0 && s.pack_current > 0.0 || s.soc <= 5.0 && s.pack_current < 0.0 {
        s.pack_current = 0.0;
    }
    s.pack_voltage = cell_volts(s.soc) * BATTERY_CELLS as f64 + s.pack_current * 0.02;
    approach(&mut s.pack_temp, 24.0 + s.pack_current.abs() * 0.15, 0.05, dt);
}

/// Cells in the simulated pack.
pub const BATTERY_CELLS: usize = 64;

/// Open-circuit cell voltage of an NMC cell at a state of charge (rough, monotonic).
fn cell_volts(soc: f64) -> f64 {
    3.3 + 0.9 * (soc / 100.0) - 0.08 * (1.0 - soc / 100.0).powi(3)
}

/// Voltage of cell `i` (0-based): the pack voltage per cell with a fixed spread of a few mV.
fn cell_voltage(s: &SimState, i: usize) -> f64 {
    let base = s.pack_voltage / BATTERY_CELLS as f64;
    base + ((i as f64 * 2.399).sin() * 0.006) + ((i as f64 * 0.731).cos() * 0.003)
}

fn build(dbc: &Dbc, name: &str, s: &SimState, pages: &mut Pages) -> Option<CanFrame> {
    let pump_page = &mut pages.pump;
    let wobble = |amp: f64| (s.hours * 3600.0 * 7.0).sin() * amp;
    let values: Vec<(&str, f64)> = match name {
        "EEC1" => vec![
            ("EngTorqueMode", if s.throttle > 0.5 { 1.0 } else { 0.0 }),
            ("DriverDemandTorque", if s.running { s.throttle } else { 0.0 }),
            ("ActualTorque", if s.running { s.throttle * 0.95 + wobble(0.8) } else { 0.0 }),
            ("EngineSpeed", (s.rpm + if s.running { wobble(3.0) } else { 0.0 }).max(0.0)),
        ],
        "ET1" => vec![("CoolantTemp", s.coolant), ("FuelTemp", s.coolant * 0.55 + 10.0), ("OilTemp", s.coolant + 8.0)],
        "EFLP1" => vec![("OilPressure", s.oil_pressure)],
        "HOURS" => vec![("EngineHours", s.hours)],
        "DM1" => {
            if s.fault {
                vec![("MalfunctionLamp", 0.0), ("AmberLamp", 1.0), ("SPN", 110.0), ("FMI", 0.0), ("OccurrenceCount", 1.0)]
            } else {
                vec![("MalfunctionLamp", 0.0), ("AmberLamp", 0.0), ("SPN", 0.0), ("FMI", 0.0), ("OccurrenceCount", 0.0)]
            }
        }
        "GenStatus" => {
            let state = if !s.running { 0.0 } else if s.fault { 5.0 } else if s.coolant < 60.0 { 2.0 } else { 3.0 };
            vec![
                ("Voltage", if s.running { 400.0 * (s.rpm / 1500.0).min(1.05) + wobble(0.6) } else { 0.0 }),
                ("Frequency", s.frequency),
                ("Current", if s.frequency > 1.0 { s.load_kw * 1000.0 / (1.732 * 400.0 * 0.9) } else { 0.0 }),
                ("Breaker", if s.breaker_closed { 1.0 } else { 0.0 }),
                ("GenState", state),
            ]
        }
        "GenPower" => vec![("ActivePower", s.load_kw), ("PowerFactor", if s.load_kw > 0.0 { 0.9 } else { 1.0 })],
        "PumpDrive" => {
            *pump_page ^= 1;
            if *pump_page == 0 {
                vec![("Mode", 0.0), ("Speed", s.pump_speed), ("Flow", s.pump_speed * 0.12)]
            } else {
                vec![("Mode", 1.0), ("MotorTemp", 35.0 + s.pump_speed * 0.01), ("FaultCode", 0.0)]
            }
        }
        "BatteryPackFD" => {
            if !s.fd_battery {
                return None;
            }
            let page = pages.cells;
            pages.cells = (pages.cells + 1) % 4;
            let cells: Vec<f64> = (0..BATTERY_CELLS).map(|i| cell_voltage(s, i)).collect();
            let (imin, vmin) = cells.iter().enumerate().fold((0, f64::MAX), |a, (i, v)| if *v < a.1 { (i, *v) } else { a });
            let (imax, vmax) = cells.iter().enumerate().fold((0, f64::MIN), |a, (i, v)| if *v > a.1 { (i, *v) } else { a });
            let state = if s.pack_current > 0.5 { 1.0 } else if s.pack_current < -0.5 { 2.0 } else { 0.0 };
            let mut v = vec![
                ("CellPage", page as f64),
                ("PackVoltage", s.pack_voltage),
                ("PackCurrent", s.pack_current),
                ("SOC", s.soc),
                ("SOH", 96.5),
                ("TempMin", s.pack_temp - 1.5),
                ("TempMax", s.pack_temp + 2.0),
                ("CellVoltMin", vmin),
                ("CellVoltMax", vmax),
                ("PackState", state),
                ("BalancingActive", if vmax - vmin > 0.012 && s.pack_current > 0.0 { 1.0 } else { 0.0 }),
                ("ContactorClosed", 1.0),
                ("CellMinIndex", (imin + 1) as f64),
                ("CellMaxIndex", (imax + 1) as f64),
                ("InsulationRes", 4800.0),
            ];
            let names: Vec<String> = (0..16).map(|k| format!("Cell{:02}", page * 16 + k + 1)).collect();
            let mut map: std::collections::HashMap<String, f64> = v.drain(..).map(|(k, x)| (k.to_string(), x)).collect();
            for (k, n) in names.into_iter().enumerate() {
                map.insert(n, cells[page * 16 + k]);
            }
            let msg = dbc.message_by_name(name)?;
            let data = encode_message(msg, &map, None).ok()?;
            return CanFrame::fd(msg.id, msg.extended, msg.brs, &data).ok();
        }
        "heartbeat" => return CanFrame::new(0x700 + CANOPEN_NODE as u32, &[0x05]).ok(),
        "tpdo1" => {
            // Digital inputs (running, breaker, fault) and one analog input (coolant, 0.1 degC).
            let di = (s.running as u8) | ((s.breaker_closed as u8) << 1) | ((s.fault as u8) << 2);
            let ai = (s.coolant * 10.0).round().clamp(0.0, 65535.0) as u16;
            return CanFrame::new(0x180 + CANOPEN_NODE as u32, &[di, 0, ai as u8, (ai >> 8) as u8]).ok();
        }
        _ => return None,
    };
    let msg = dbc.message_by_name(name)?;
    let map = values.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    let data = encode_message(msg, &map, Some(&[0xFF; 8])).ok()?;
    CanFrame::with_ext(msg.id, msg.extended, &data).ok()
}

/// Answers OBD-II requests and CANopen SDO uploads, and takes commands.
async fn handle_rx(dbc: &Dbc, f: &CanFrame, state: &Arc<Mutex<SimState>>) -> Option<CanFrame> {
    if f.rtr {
        return None;
    }
    if !f.extended && (f.id == 0x7DF || f.id == 0x7E0) && f.data.len() >= 3 && f.data[1] == 0x01 {
        let s = state.lock().await;
        let pid = f.data[2];
        let payload: Vec<u8> = match pid {
            0x00 => vec![0x18, 0x18, 0x80, 0x02], // 04 05 0C 0D 11 2F supported
            0x04 => vec![(s.throttle * 2.55).round() as u8],
            0x05 => vec![(s.coolant + 40.0).clamp(0.0, 255.0) as u8],
            0x0C => {
                let v = (s.rpm * 4.0).round().clamp(0.0, 65535.0) as u16;
                vec![(v >> 8) as u8, v as u8]
            }
            0x0D => vec![0],
            0x11 => vec![(s.throttle * 2.55).round() as u8],
            0x2F => vec![200],
            _ => return None,
        };
        let mut d = vec![(2 + payload.len()) as u8, 0x41, pid];
        d.extend(payload);
        d.resize(8, 0x55);
        return CanFrame::new(0x7E8, &d).ok();
    }
    if !f.extended && f.id == 0x600 + CANOPEN_NODE as u32 && f.data.len() == 8 && f.data[0] == 0x40 {
        // Expedited upload of a few identity objects.
        let index = u16::from_le_bytes([f.data[1], f.data[2]]);
        let sub = f.data[3];
        let value: Option<u32> = match (index, sub) {
            (0x1000, 0) => Some(0x0004_0191), // device type: CiA 401 I/O
            (0x1018, 1) => Some(0x0000_0A5A), // vendor id
            (0x1018, 2) => Some(0x0000_0401), // product code
            (0x1018, 3) => Some(0x0001_0000), // revision
            (0x1017, 0) => Some(1000),        // producer heartbeat time
            _ => None,
        };
        let mut d = [0u8; 8];
        d[1..4].copy_from_slice(&f.data[1..4]);
        match value {
            Some(v) => {
                d[0] = 0x43;
                d[4..].copy_from_slice(&v.to_le_bytes());
            }
            None => {
                d[0] = 0x80;
                d[4..].copy_from_slice(&0x0602_0000u32.to_le_bytes()); // object does not exist
            }
        }
        return CanFrame::new(0x580 + CANOPEN_NODE as u32, &d).ok();
    }
    if let Some((msg, values)) = dbc.decode(f) {
        let mut s = state.lock().await;
        let get = |n: &str| values.iter().find(|v| v.name == n).map(|v| v.value);
        match msg.name.as_str() {
            "ThrottleCommand" => {
                if let Some(t) = get("Throttle") {
                    s.throttle = t.clamp(0.0, 100.0);
                }
            }
            "PumpCommand" => {
                if let Some(sp) = get("SpeedSetpoint") {
                    s.pump_setpoint = sp.clamp(0.0, 3000.0);
                }
                if let Some(e) = get("Enable") {
                    s.pump_enabled = e >= 0.5;
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Direction;

    #[test]
    fn built_in_dbc_parses() {
        let d = Dbc::parse(SIM_DBC).unwrap();
        assert_eq!(d.messages.len(), 11);
        let b = d.message_by_name("BatteryPackFD").unwrap();
        assert_eq!((b.id, b.extended, b.size, b.fd, b.brs, b.cycle_ms), (0x4A0, false, 64, true, true, Some(100)));
        assert_eq!(b.signals.len(), 15 + BATTERY_CELLS);
        // The classic messages stay classic.
        assert!(!d.message_by_name("EEC1").unwrap().fd);
        assert!(!d.message_by_name("PumpDrive").unwrap().brs);
        assert!(d.message(0x0CF0_0400, true).is_some());
        assert_eq!(d.message(0x18FE_EE00, true).unwrap().name, "ET1");
        assert_eq!(d.message_by_name("PumpDrive").unwrap().cycle_ms, Some(50));
    }

    #[tokio::test]
    async fn fd_battery_pack_frames_and_toggle() {
        let bus = VirtualBus::new();
        let sim = Simulator::start(&bus);
        let mut me = bus.attach();
        let mut ev = me.take_events().unwrap();
        let dbc = Dbc::parse(SIM_DBC).unwrap();
        let mut pages = std::collections::BTreeSet::new();
        let mut classic_eec1 = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline && (pages.len() < 4 || !classic_eec1) {
            let Ok(Some(BusEvent::Frame(f))) = tokio::time::timeout(Duration::from_millis(500), ev.recv()).await else { continue };
            if f.frame.extended && f.frame.id == 0x0CF0_0400 {
                assert!(!f.frame.fd, "J1939 traffic stays classic CAN");
                classic_eec1 = true;
            }
            if !f.frame.extended && f.frame.id == 0x4A0 {
                assert!(f.frame.fd && f.frame.brs && f.frame.data.len() == 64);
                let (m, v) = dbc.decode(&f.frame).unwrap();
                assert_eq!(m.name, "BatteryPackFD");
                let get = |n: &str| v.iter().find(|s| s.name == n).map(|s| s.value);
                let page = get("CellPage").unwrap() as usize;
                let pack = get("PackVoltage").unwrap();
                assert!((200.0..300.0).contains(&pack), "pack voltage {pack}");
                assert!((0.0..=100.0).contains(&get("SOC").unwrap()));
                // This page's 16 cells decode, and only those.
                let cells: Vec<f64> = (0..16).map(|k| get(&format!("Cell{:02}", page * 16 + k + 1)).unwrap()).collect();
                assert!(cells.iter().all(|c| (3.0..4.4).contains(c)), "{cells:?}");
                assert!(get(&format!("Cell{:02}", ((page + 1) % 4) * 16 + 1)).is_none());
                let min = get("CellVoltMin").unwrap();
                assert!(cells.iter().all(|c| *c >= min - 0.0011));
                pages.insert(page);
            }
        }
        assert_eq!(pages.len(), 4, "all four cell pages went out: {pages:?}");
        assert!(classic_eec1);

        sim.apply(SimAction::SetFdBattery { enabled: false }).await;
        assert!(!sim.state().await.fd_battery);
        tokio::time::sleep(Duration::from_millis(150)).await;
        while ev.try_recv().is_ok() {}
        let quiet_until = tokio::time::Instant::now() + Duration::from_millis(400);
        while let Ok(Some(e)) = tokio::time::timeout_at(quiet_until, ev.recv()).await {
            if let BusEvent::Frame(f) = e {
                assert!(f.frame.id != 0x4A0 || f.frame.extended, "battery frame sent while switched off");
            }
        }
        sim.stop().await;
    }

    #[tokio::test]
    async fn simulator_sends_and_answers() {
        let bus = VirtualBus::new();
        let sim = Simulator::start(&bus);
        let mut me = bus.attach();
        let mut ev = me.take_events().unwrap();
        let dbc = Dbc::parse(SIM_DBC).unwrap();
        let mut seen_eec1 = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        me.send(CanFrame::new(0x7DF, &[2, 1, 0x0C, 0, 0, 0, 0, 0]).unwrap()).await.unwrap();
        me.send(CanFrame::new(0x611, &[0x40, 0x18, 0x10, 0x01, 0, 0, 0, 0]).unwrap()).await.unwrap();
        let thr = dbc.message_by_name("ThrottleCommand").unwrap();
        let data = encode_message(thr, &[("Throttle".to_string(), 80.0)].into_iter().collect(), None).unwrap();
        me.send(CanFrame::new(thr.id, &data).unwrap()).await.unwrap();
        let (mut obd, mut sdo) = (false, false);
        while tokio::time::Instant::now() < deadline && !(seen_eec1 && obd && sdo) {
            let Ok(Some(BusEvent::Frame(f))) = tokio::time::timeout(Duration::from_millis(500), ev.recv()).await else { continue };
            if f.dir == Direction::Tx {
                continue;
            }
            if f.frame.extended && f.frame.id == 0x0CF0_0400 {
                seen_eec1 = true;
            }
            if f.frame.id == 0x7E8 {
                assert_eq!(&f.frame.data[..3], &[4, 0x41, 0x0C]);
                obd = true;
            }
            if f.frame.id == 0x591 {
                assert_eq!(f.frame.data[0], 0x43);
                assert_eq!(&f.frame.data[4..], &0x0A5Au32.to_le_bytes());
                sdo = true;
            }
        }
        assert!(seen_eec1 && obd && sdo, "eec1 {seen_eec1} obd {obd} sdo {sdo}");
        assert_eq!(sim.state().await.throttle, 80.0);
        sim.apply(SimAction::SetFault { active: true }).await;
        assert!(sim.state().await.fault);
        sim.stop().await;
    }
}
