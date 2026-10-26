//! The plant the OPC UA simulator serves: a batch reactor, a steam boiler and the site's utilities,
//! with methods to start a batch and reset alarms, and how their values move.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use mt_opcua::{MethodHandler, PlantArgument, PlantMethod, PlantNode, PlantVariable};
use tokio::sync::mpsc;

/// Whether the simulation drives a variable or only stores what it is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// The simulation moves it; a write holds it at the written value.
    Measured,
    /// An input to the simulation (a setpoint, a command); writes take effect.
    Setting,
}

/// One variable of the plant, and one point of the simulator session.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Variable {
    pub key: &'static str,
    pub name: &'static str,
    pub data_type: &'static str,
    pub unit: &'static str,
    pub range: Option<(f64, f64)>,
    pub kind: Kind,
    /// Clients may write it.
    pub writable: bool,
}

const fn var(
    key: &'static str,
    name: &'static str,
    data_type: &'static str,
    unit: &'static str,
    range: Option<(f64, f64)>,
    kind: Kind,
    writable: bool,
) -> Variable {
    Variable {
        key,
        name,
        data_type,
        unit,
        range,
        kind,
        writable,
    }
}

pub(crate) const TEMPERATURE: usize = 0;
pub(crate) const SETPOINT: usize = 1;
pub(crate) const PRESSURE: usize = 2;
pub(crate) const LEVEL: usize = 3;
pub(crate) const AGITATOR: usize = 4;
pub(crate) const RUNNING: usize = 5;
pub(crate) const HIGH_TEMPERATURE: usize = 6;
pub(crate) const RECIPE: usize = 7;
pub(crate) const BATCH: usize = 8;
pub(crate) const STEAM_PRESSURE: usize = 9;
pub(crate) const DRUM_LEVEL: usize = 10;
pub(crate) const FEEDWATER: usize = 11;
pub(crate) const FLUE_GAS: usize = 12;
pub(crate) const EFFICIENCY: usize = 13;
pub(crate) const BURNER: usize = 14;
pub(crate) const POWER: usize = 15;
pub(crate) const ENERGY: usize = 16;

use Kind::{Measured, Setting};

/// Every variable, in point order.
pub(crate) const VARIABLES: [Variable; 17] = [
    var(
        "Plant.Reactor.Temperature",
        "Temperature",
        "Double",
        "°C",
        Some((0.0, 200.0)),
        Measured,
        true,
    ),
    var(
        "Plant.Reactor.Setpoint",
        "Setpoint",
        "Double",
        "°C",
        Some((20.0, 180.0)),
        Setting,
        true,
    ),
    var(
        "Plant.Reactor.Pressure",
        "Pressure",
        "Double",
        "bar",
        Some((0.0, 10.0)),
        Measured,
        true,
    ),
    var(
        "Plant.Reactor.Level",
        "Level",
        "Double",
        "%",
        Some((0.0, 100.0)),
        Measured,
        true,
    ),
    var(
        "Plant.Reactor.AgitatorSpeed",
        "AgitatorSpeed",
        "Double",
        "rpm",
        Some((0.0, 600.0)),
        Setting,
        true,
    ),
    var(
        "Plant.Reactor.Running",
        "Running",
        "Boolean",
        "",
        None,
        Setting,
        true,
    ),
    var(
        "Plant.Reactor.HighTemperature",
        "HighTemperature",
        "Boolean",
        "",
        None,
        Measured,
        false,
    ),
    var(
        "Plant.Reactor.Recipe",
        "Recipe",
        "String",
        "",
        None,
        Setting,
        true,
    ),
    var(
        "Plant.Reactor.BatchId",
        "BatchId",
        "UInt32",
        "",
        None,
        Measured,
        false,
    ),
    var(
        "Plant.Boiler.SteamPressure",
        "SteamPressure",
        "Double",
        "bar",
        Some((0.0, 25.0)),
        Measured,
        true,
    ),
    var(
        "Plant.Boiler.DrumLevel",
        "DrumLevel",
        "Double",
        "%",
        Some((0.0, 100.0)),
        Measured,
        true,
    ),
    var(
        "Plant.Boiler.FeedwaterFlow",
        "FeedwaterFlow",
        "Double",
        "m³/h",
        Some((0.0, 40.0)),
        Measured,
        true,
    ),
    var(
        "Plant.Boiler.FlueGasTemperature",
        "FlueGasTemperature",
        "Double",
        "°C",
        Some((0.0, 400.0)),
        Measured,
        true,
    ),
    var(
        "Plant.Boiler.Efficiency",
        "Efficiency",
        "Double",
        "%",
        Some((0.0, 100.0)),
        Measured,
        true,
    ),
    var(
        "Plant.Boiler.BurnerOn",
        "BurnerOn",
        "Boolean",
        "",
        None,
        Setting,
        true,
    ),
    var(
        "Plant.Utilities.PowerDemand",
        "PowerDemand",
        "Double",
        "kW",
        Some((0.0, 1000.0)),
        Measured,
        true,
    ),
    var(
        "Plant.Utilities.EnergyTotal",
        "EnergyTotal",
        "Double",
        "kWh",
        None,
        Measured,
        false,
    ),
];

pub(crate) const START_BATCH: &str = "Plant.Reactor.StartBatch";
pub(crate) const RESET_ALARMS: &str = "Plant.Reactor.ResetAlarms";
/// The reactor vessel's working volume: a batch's quantity fills it this far.
const VESSEL_LITRES: f64 = 2000.0;
const AMBIENT: f64 = 22.0;
const ALARM_ABOVE: f64 = 150.0;
const FIRST_BATCH: u32 = 1001;

fn initial(index: usize) -> f64 {
    match index {
        TEMPERATURE => 64.0,
        SETPOINT => 85.0,
        PRESSURE => 2.3,
        LEVEL => 72.0,
        AGITATOR => 240.0,
        RUNNING | BURNER => 1.0,
        BATCH => f64::from(FIRST_BATCH),
        STEAM_PRESSURE => 12.0,
        DRUM_LEVEL => 50.0,
        FEEDWATER => 18.0,
        FLUE_GAS => 185.0,
        EFFICIENCY => 88.0,
        POWER => 420.0,
        ENERGY => 15_230.0,
        _ => 0.0,
    }
}

fn plant_variable(index: usize) -> PlantNode {
    let variable = VARIABLES[index];
    PlantNode::Variable(PlantVariable {
        key: variable.key.to_owned(),
        name: variable.name.to_owned(),
        data_type: variable.data_type.to_owned(),
        unit: variable.unit.to_owned(),
        range: variable.range,
        writable: variable.writable,
        initial: if index == RECIPE {
            serde_json::Value::String(INITIAL_RECIPE.to_owned())
        } else {
            json(variable.data_type, initial(index))
        },
    })
}

const INITIAL_RECIPE: &str = "Base resin";

/// A number as the JSON of a variable's type: booleans from 0 and 1.
pub(crate) fn json(data_type: &str, value: f64) -> serde_json::Value {
    match data_type {
        "Boolean" => serde_json::Value::Bool(value >= 0.5),
        "UInt32" => serde_json::json!(value.max(0.0).round() as u64),
        _ => serde_json::json!(value),
    }
}

fn argument(name: &str, data_type: &str, description: &str) -> PlantArgument {
    PlantArgument {
        name: name.to_owned(),
        data_type: data_type.to_owned(),
        description: description.to_owned(),
    }
}

fn machine(key: &str, name: &str, children: Vec<PlantNode>) -> PlantNode {
    PlantNode::Object {
        key: key.to_owned(),
        name: name.to_owned(),
        children,
    }
}

/// The address space under Objects.
pub(crate) fn nodes() -> Vec<PlantNode> {
    let range = |from: usize, to: usize| (from..=to).map(plant_variable).collect::<Vec<_>>();
    let mut reactor = range(TEMPERATURE, BATCH);
    reactor.push(PlantNode::Method(PlantMethod {
        key: START_BATCH.to_owned(),
        name: "StartBatch".to_owned(),
        inputs: vec![
            argument("Recipe", "String", "Name of the recipe to run"),
            argument("Quantity", "Double", "Litres to charge, up to 2000"),
        ],
        outputs: vec![argument("BatchId", "UInt32", "Number of the started batch")],
    }));
    reactor.push(PlantNode::Method(PlantMethod {
        key: RESET_ALARMS.to_owned(),
        name: "ResetAlarms".to_owned(),
        inputs: Vec::new(),
        outputs: vec![argument("Cleared", "UInt32", "Alarms that were cleared")],
    }));
    vec![PlantNode::Folder {
        key: "Plant".to_owned(),
        name: "Plant".to_owned(),
        children: vec![
            machine("Plant.Reactor", "Reactor", reactor),
            machine("Plant.Boiler", "Boiler", range(STEAM_PRESSURE, BURNER)),
            machine("Plant.Utilities", "Utilities", range(POWER, ENERGY)),
        ],
    }]
}

/// What a method call asks of the simulation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Effect {
    StartBatch {
        recipe: String,
        litres: f64,
        batch: u32,
    },
    ResetAlarms,
}

/// State the method handler (on the server's threads) shares with the simulation.
#[derive(Debug)]
pub(crate) struct Shared {
    next_batch: AtomicU32,
    active_alarms: AtomicU32,
}

impl Shared {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            next_batch: AtomicU32::new(FIRST_BATCH + 1),
            active_alarms: AtomicU32::new(0),
        })
    }
}

/// Validate a call, answer it and hand its effect to the simulation.
pub(crate) fn call(
    shared: &Shared,
    method: &str,
    inputs: &[serde_json::Value],
) -> Result<(Vec<serde_json::Value>, Effect), String> {
    match method {
        START_BATCH => {
            let recipe = inputs
                .first()
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|recipe| !recipe.is_empty())
                .ok_or("Name the recipe")?;
            let litres = inputs
                .get(1)
                .and_then(serde_json::Value::as_f64)
                .filter(|litres| *litres > 0.0 && *litres <= VESSEL_LITRES)
                .ok_or("Charge between 0 and 2000 litres")?;
            let batch = shared.next_batch.fetch_add(1, Ordering::Relaxed);
            Ok((
                vec![serde_json::json!(batch)],
                Effect::StartBatch {
                    recipe: recipe.to_owned(),
                    litres,
                    batch,
                },
            ))
        }
        RESET_ALARMS => {
            let cleared = shared.active_alarms.load(Ordering::Relaxed);
            Ok((vec![serde_json::json!(cleared)], Effect::ResetAlarms))
        }
        other => Err(format!("No method {other}")),
    }
}

/// The method handler the server calls: answers and forwards the effect to the simulation.
pub(crate) fn handler(
    shared: Arc<Shared>,
    effects: mpsc::UnboundedSender<Effect>,
) -> MethodHandler {
    Arc::new(move |method: &str, inputs: &[serde_json::Value]| {
        let (outputs, effect) = call(&shared, method, inputs)?;
        let _ = effects.send(effect);
        Ok(outputs)
    })
}

/// The simulated plant's values.
pub(crate) struct Plant {
    pub values: [f64; VARIABLES.len()],
    pub recipe: String,
    /// Seconds simulated.
    time: f64,
    seed: u32,
}

impl Plant {
    pub(crate) fn new() -> Self {
        Self {
            values: std::array::from_fn(initial),
            recipe: INITIAL_RECIPE.to_owned(),
            time: 0.0,
            seed: 0x5EED_0A0A,
        }
    }

    /// Uniform noise in -1..1.
    fn noise(&mut self) -> f64 {
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        f64::from(x) / f64::from(u32::MAX) * 2.0 - 1.0
    }

    /// Advance `dt` seconds. Held measured values stay where they are.
    pub(crate) fn step(&mut self, dt: f64, held: &[bool], shared: &Shared) {
        self.time += dt;
        let t = self.time;
        let mut next = self.values;
        let running = next[RUNNING] >= 0.5;
        let burner = next[BURNER] >= 0.5;

        let target = if running { next[SETPOINT] } else { AMBIENT };
        next[TEMPERATURE] +=
            (target - next[TEMPERATURE]) * (dt / 20.0).min(1.0) + self.noise() * 0.08;
        // A held temperature is the one the rest of the reactor sees.
        let temperature = if held.get(TEMPERATURE).copied().unwrap_or(false) {
            self.values[TEMPERATURE]
        } else {
            next[TEMPERATURE]
        };
        next[PRESSURE] = 1.0 + (temperature - 20.0).max(0.0) * 0.03 + self.noise() * 0.02;
        if running {
            next[LEVEL] -= dt * 0.08 * (0.5 + next[AGITATOR] / 600.0);
            if next[LEVEL] <= 5.0 {
                next[LEVEL] = 5.0;
                // The batch is done: the reactor stops.
                next[RUNNING] = 0.0;
            }
        }
        if temperature > ALARM_ABOVE {
            next[HIGH_TEMPERATURE] = 1.0;
        }

        let steam = if burner {
            12.0 + 1.5 * (t / 40.0).sin()
        } else {
            0.5
        };
        next[STEAM_PRESSURE] +=
            (steam - next[STEAM_PRESSURE]) * (dt / 15.0).min(1.0) + self.noise() * 0.03;
        next[DRUM_LEVEL] = 50.0 + 4.0 * (t / 23.0).sin() + self.noise() * 0.3;
        next[FEEDWATER] = 18.0 + 3.0 * (t / 31.0 + 1.0).sin() + self.noise() * 0.2;
        let flue = if burner {
            185.0 + 10.0 * (t / 50.0).sin()
        } else {
            40.0
        };
        next[FLUE_GAS] += (flue - next[FLUE_GAS]) * (dt / 30.0).min(1.0) + self.noise() * 0.2;
        next[EFFICIENCY] = if burner {
            88.0 + 1.5 * (t / 60.0).sin() + self.noise() * 0.1
        } else {
            0.0
        };

        next[POWER] =
            420.0 + 80.0 * (t / 45.0).sin() + if running { 35.0 } else { 0.0 } + self.noise() * 6.0;
        next[ENERGY] += next[POWER] * dt / 3600.0;

        for (index, variable) in VARIABLES.iter().enumerate() {
            let keep = variable.kind == Setting || held.get(index).copied().unwrap_or(false);
            // Settings change only from writes and method calls, but a finished batch stops the reactor.
            if !keep || (index == RUNNING && next[RUNNING] < 0.5) {
                self.values[index] = next[index];
            }
        }
        if let Some((low, high)) = VARIABLES[TEMPERATURE].range
            && !held[TEMPERATURE]
        {
            self.values[TEMPERATURE] = self.values[TEMPERATURE].clamp(low, high);
        }
        shared.active_alarms.store(
            u32::from(self.values[HIGH_TEMPERATURE] >= 0.5),
            Ordering::Relaxed,
        );
    }

    pub(crate) fn apply(&mut self, effect: &Effect) {
        match effect {
            Effect::StartBatch {
                recipe,
                litres,
                batch,
            } => {
                self.recipe.clone_from(recipe);
                self.values[BATCH] = f64::from(*batch);
                self.values[LEVEL] = (litres / VESSEL_LITRES * 100.0).clamp(5.0, 100.0);
                self.values[RUNNING] = 1.0;
            }
            Effect::ResetAlarms => {
                if self.values[TEMPERATURE] <= ALARM_ABOVE {
                    self.values[HIGH_TEMPERATURE] = 0.0;
                }
            }
        }
    }

    /// Every variable's value as the JSON of its type.
    pub(crate) fn json_values(&self) -> Vec<serde_json::Value> {
        VARIABLES
            .iter()
            .enumerate()
            .map(|(index, variable)| {
                if index == RECIPE {
                    serde_json::Value::String(self.recipe.clone())
                } else {
                    json(variable.data_type, self.values[index])
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batch_charges_the_reactor_and_runs_it_down() {
        let shared = Shared::new();
        let mut plant = Plant::new();
        let held = [false; VARIABLES.len()];
        let (outputs, effect) = call(
            &shared,
            START_BATCH,
            &[serde_json::json!("Pale ale"), serde_json::json!(500.0)],
        )
        .unwrap();
        assert_eq!(outputs, vec![serde_json::json!(FIRST_BATCH + 1)]);
        plant.apply(&effect);
        assert_eq!(plant.recipe, "Pale ale");
        assert_eq!(plant.values[LEVEL], 25.0);
        assert_eq!(plant.values[RUNNING], 1.0);
        // The level falls while running, until the batch is done and the reactor stops.
        for _ in 0..2000 {
            plant.step(1.0, &held, &shared);
        }
        assert_eq!(plant.values[LEVEL], 5.0);
        assert_eq!(
            plant.values[RUNNING], 0.0,
            "a finished batch stops the reactor"
        );
        // Stopped, the reactor cools to ambient.
        assert!((plant.values[TEMPERATURE] - AMBIENT).abs() < 1.0);
    }

    #[test]
    fn calls_with_bad_arguments_are_refused() {
        let shared = Shared::new();
        assert!(
            call(
                &shared,
                START_BATCH,
                &[serde_json::json!(""), serde_json::json!(5.0)]
            )
            .is_err()
        );
        assert!(
            call(
                &shared,
                START_BATCH,
                &[serde_json::json!("x"), serde_json::json!(2001.0)]
            )
            .is_err()
        );
        assert!(call(&shared, START_BATCH, &[serde_json::json!("x")]).is_err());
        assert!(call(&shared, "Plant.Reactor.Explode", &[]).is_err());
    }

    #[test]
    fn held_values_and_settings_stay_put() {
        let shared = Shared::new();
        let mut plant = Plant::new();
        let mut held = [false; VARIABLES.len()];
        held[PRESSURE] = true;
        plant.values[PRESSURE] = 9.5;
        plant.values[SETPOINT] = 120.0;
        for _ in 0..10 {
            plant.step(0.5, &held, &shared);
        }
        assert_eq!(plant.values[PRESSURE], 9.5);
        assert_eq!(plant.values[SETPOINT], 120.0);
        assert!(
            plant.values[TEMPERATURE] > 64.0,
            "heating towards the new setpoint"
        );
    }

    #[test]
    fn the_alarm_latches_until_reset_below_the_limit() {
        let shared = Shared::new();
        let mut plant = Plant::new();
        let mut held = [false; VARIABLES.len()];
        held[TEMPERATURE] = true;
        plant.values[TEMPERATURE] = 160.0;
        plant.step(0.5, &held, &shared);
        assert_eq!(plant.values[HIGH_TEMPERATURE], 1.0);
        let (cleared, effect) = call(&shared, RESET_ALARMS, &[]).unwrap();
        assert_eq!(cleared, vec![serde_json::json!(1)]);
        plant.apply(&effect);
        assert_eq!(
            plant.values[HIGH_TEMPERATURE], 1.0,
            "still too hot to clear"
        );
        plant.values[TEMPERATURE] = 100.0;
        plant.step(0.5, &held, &shared);
        assert_eq!(plant.values[HIGH_TEMPERATURE], 1.0, "latched");
        plant.apply(&effect);
        assert_eq!(plant.values[HIGH_TEMPERATURE], 0.0);
    }
}
