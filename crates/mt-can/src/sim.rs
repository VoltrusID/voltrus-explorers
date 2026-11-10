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

BU_: ECU GEN PUMP IO Tester

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
CM_ BO_ 1040 "Cooling pump drive. Mode 0: speed and flow, mode 1: motor temperature and fault code.";
CM_ BO_ 1296 "Send this to the simulator to set the pump speed.";
CM_ BO_ 1297 "Send this to the simulator to set the engine throttle.";
CM_ SG_ 2364539904 EngineSpeed "Actual engine speed.";
CM_ SG_ 801 Breaker "Generator breaker position.";
BA_DEF_ BO_ "GenMsgCycleTime" INT 0 65535;
BA_ "GenMsgCycleTime" BO_ 2364539904 20;
BA_ "GenMsgCycleTime" BO_ 2566843904 1000;
BA_ "GenMsgCycleTime" BO_ 2566844160 500;
BA_ "GenMsgCycleTime" BO_ 2566841600 1000;
BA_ "GenMsgCycleTime" BO_ 2566834688 1000;
BA_ "GenMsgCycleTime" BO_ 801 100;
BA_ "GenMsgCycleTime" BO_ 802 100;
BA_ "GenMsgCycleTime" BO_ 1040 50;
VAL_ 2364539904 EngTorqueMode 0 "Low idle governor" 1 "Accelerator pedal" 2 "Cruise control" 3 "PTO governor" 15 "Not available" ;
VAL_ 801 Breaker 0 "Open" 1 "Closed" 2 "Tripped" 3 "Not available" ;
VAL_ 801 GenState 0 "Stopped" 1 "Starting" 2 "Warm-up" 3 "Running" 4 "Cool-down" 5 "Fault" ;
VAL_ 1040 Mode 0 "Process data" 1 "Diagnostics" ;
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
    let mut pump_page = 0u8;
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
                            if let Some(f) = build(&dbc, &e.name, &s, &mut pump_page) {
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
}

fn build(dbc: &Dbc, name: &str, s: &SimState, pump_page: &mut u8) -> Option<CanFrame> {
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
        assert_eq!(d.messages.len(), 10);
        assert!(d.message(0x0CF0_0400, true).is_some());
        assert_eq!(d.message(0x18FE_EE00, true).unwrap().name, "ET1");
        assert_eq!(d.message_by_name("PumpDrive").unwrap().cycle_ms, Some(50));
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
