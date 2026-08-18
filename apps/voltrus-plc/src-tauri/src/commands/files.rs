use std::path::Path;

use crate::commands::io::{IoDriverConfig, IoMapping};


#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct SampleTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub source: String,
}

#[tauri::command]
pub async fn new_file(name: String, pou_type: String) -> Result<String, String> {
    let template = match pou_type.as_str() {
        "program" => format!(
            "PROGRAM {}\nVAR\n    \nEND_VAR\n\nEND_PROGRAM",
            name
        ),
        "function" => format!(
            "FUNCTION {} : INT\nVAR_INPUT\n    \nEND_VAR\n\n    {} := 0;\nEND_FUNCTION",
            name, name
        ),
        "function_block" => format!(
            "FUNCTION_BLOCK {}\nVAR_INPUT\n    \nEND_VAR\n\nEND_FUNCTION_BLOCK",
            name
        ),
        _ => return Err("Unknown POU type".into()),
    };
    Ok(template)
}

#[tauri::command]
pub async fn open_file(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::read_to_string(&path).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn save_file(path: String, content: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::write(&path, &content).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_project_files(dir: String) -> Result<Vec<FileEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || list_project_files_blocking(&dir))
        .await
        .map_err(|e| e.to_string())?
}

fn list_project_files_blocking(dir: &str) -> Result<Vec<FileEntry>, String> {
    let path = Path::new(&dir);
    if !path.exists() {
        return Ok(vec![]);
    }

    let mut entries = Vec::new();
    let read_dir = std::fs::read_dir(path).map_err(|e| e.to_string())?;

    for entry in read_dir {
        let entry = entry.map_err(|e| e.to_string())?;
        let file_name = entry.file_name().to_string_lossy().to_string();
        if file_name.starts_with('.') {
            continue;
        }
        let metadata = entry.metadata().map_err(|e| e.to_string())?;
        entries.push(FileEntry {
            name: file_name,
            path: entry.path().to_string_lossy().to_string(),
            is_dir: metadata.is_dir(),
        });
    }

    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}

#[tauri::command]
pub async fn get_sample_templates() -> Vec<SampleTemplate> {
    vec![
        SampleTemplate {
            id: "counter".into(),
            name: "Counter".into(),
            description: "Bidirectional counter with direction switching".into(),
            category: "Basic".into(),
            source: r#"PROGRAM Counter
VAR
    counter : INT := 0;
    max_count : INT := 100;
    done : BOOL := FALSE;
    cycles : DINT := 0;
    direction : INT := 1;
END_VAR
    cycles := cycles + 1;
    counter := counter + direction;
    IF counter >= max_count THEN
        direction := -1;
        done := TRUE;
    ELSIF counter <= 0 THEN
        direction := 1;
        done := FALSE;
    END_IF;
END_PROGRAM"#.into(),
        },
        SampleTemplate {
            id: "traffic-light".into(),
            name: "Traffic Light".into(),
            description: "4-state traffic light controller with timed transitions".into(),
            category: "Basic".into(),
            source: r#"PROGRAM TrafficLight
VAR
    state : INT := 0;
    timer : INT := 0;
    red : BOOL := TRUE;
    yellow : BOOL := FALSE;
    green : BOOL := FALSE;
END_VAR
    timer := timer + 1;

    IF state = 0 THEN
        red := TRUE; yellow := FALSE; green := FALSE;
        IF timer > 100 THEN state := 1; timer := 0; END_IF;
    ELSIF state = 1 THEN
        red := TRUE; yellow := TRUE; green := FALSE;
        IF timer > 30 THEN state := 2; timer := 0; END_IF;
    ELSIF state = 2 THEN
        red := FALSE; yellow := FALSE; green := TRUE;
        IF timer > 100 THEN state := 3; timer := 0; END_IF;
    ELSIF state = 3 THEN
        red := FALSE; yellow := TRUE; green := FALSE;
        IF timer > 40 THEN state := 0; timer := 0; END_IF;
    END_IF;
END_PROGRAM"#.into(),
        },
        SampleTemplate {
            id: "motor-control".into(),
            name: "Motor Control".into(),
            description: "Motor with start/stop, emergency, overload and speed ramping".into(),
            category: "Industrial".into(),
            source: r#"PROGRAM MotorControl
VAR
    start_btn : BOOL := FALSE;
    stop_btn : BOOL := FALSE;
    emergency_stop : BOOL := FALSE;
    overload_trip : BOOL := FALSE;
    motor_running : BOOL := FALSE;
    fault : BOOL := FALSE;
    speed : INT := 0;
    fault_code : INT := 0;
    target_speed : INT := 1000;
    ramp_rate : INT := 10;
    decel_rate : INT := 25;
    run_time : DINT := 0;
END_VAR
    run_time := run_time + 1;
    IF overload_trip THEN fault := TRUE; fault_code := 1; motor_running := FALSE; END_IF;
    IF emergency_stop THEN fault := TRUE; fault_code := 2; motor_running := FALSE; END_IF;
    IF start_btn AND (NOT stop_btn) AND (NOT fault) THEN
        motor_running := TRUE;
    END_IF;
    IF stop_btn THEN motor_running := FALSE; END_IF;
    IF stop_btn AND fault THEN
        fault := FALSE; fault_code := 0;
    END_IF;
    IF motor_running AND speed < target_speed THEN
        speed := speed + ramp_rate;
        IF speed > target_speed THEN speed := target_speed; END_IF;
    ELSIF (NOT motor_running) AND speed > 0 THEN
        speed := speed - decel_rate;
        IF speed < 0 THEN speed := 0; END_IF;
    END_IF;
END_PROGRAM"#.into(),
        },
        SampleTemplate {
            id: "tank-level".into(),
            name: "Tank Level".into(),
            description: "PID-like tank level and temperature control with alarms".into(),
            category: "Process".into(),
            source: r#"PROGRAM TankLevel
VAR
    level : REAL := 25.0;
    temperature : REAL := 20.0;
    level_setpoint : REAL := 50.0;
    temp_setpoint : REAL := 65.0;
    deadband : REAL := 3.0;
    inlet_valve : BOOL := FALSE;
    outlet_valve : BOOL := FALSE;
    heater : BOOL := FALSE;
    high_level_alarm : BOOL := FALSE;
    low_level_alarm : BOOL := FALSE;
    high_temp_alarm : BOOL := FALSE;
    inlet_flow : REAL := 1.0;
    outlet_flow : REAL := 0.6;
    heat_rate : REAL := 0.3;
END_VAR
    IF level < level_setpoint - deadband THEN
        inlet_valve := TRUE; outlet_valve := FALSE;
    ELSIF level > level_setpoint + deadband THEN
        inlet_valve := FALSE; outlet_valve := TRUE;
    END_IF;
    IF temperature < temp_setpoint - deadband THEN
        heater := TRUE;
    ELSIF temperature > temp_setpoint + deadband THEN
        heater := FALSE;
    END_IF;
    high_level_alarm := level > 90.0;
    low_level_alarm := level < 10.0;
    high_temp_alarm := temperature > 85.0;
    IF high_level_alarm THEN inlet_valve := FALSE; END_IF;
    IF high_temp_alarm THEN heater := FALSE; END_IF;
    IF inlet_valve THEN level := level + inlet_flow; END_IF;
    IF outlet_valve THEN level := level - outlet_flow; END_IF;
    IF heater THEN temperature := temperature + heat_rate; END_IF;
    temperature := temperature - 0.05;
    IF level > 100.0 THEN level := 100.0; END_IF;
    IF level < 0.0 THEN level := 0.0; END_IF;
END_PROGRAM"#.into(),
        },
        SampleTemplate {
            id: "ton-timer".into(),
            name: "Timer Example".into(),
            description: "TON timer with delayed motor start".into(),
            category: "Timers".into(),
            source: r#"PROGRAM TimerExample
VAR
    start : BOOL := FALSE;
    motor : BOOL := FALSE;
    startup_delay : TON;
END_VAR
    startup_delay(IN := start, PT := T#500MS);
    motor := startup_delay.Q;
END_PROGRAM"#.into(),
        },
        SampleTemplate {
            id: "counter-fb".into(),
            name: "Counter FB Example".into(),
            description: "CTU counter counting parts with reset".into(),
            category: "Counters".into(),
            source: r#"PROGRAM CounterExample
VAR
    sensor_pulse : BOOL := FALSE;
    reset : BOOL := FALSE;
    batch_done : BOOL := FALSE;
    part_count : CTU;
END_VAR
    part_count(CU := sensor_pulse, R := reset, PV := 10);
    batch_done := part_count.Q;
END_PROGRAM"#.into(),
        },
        SampleTemplate {
            id: "esp32-blink".into(),
            name: "ESP32 Blink".into(),
            description: "Blink relay 0 every 500ms — simplest hardware test".into(),
            category: "ESP32 Hardware".into(),
            source: r#"PROGRAM ESP32_Blink
VAR
    relay0 : BOOL := FALSE;
    flash : TON;
END_VAR
    flash(IN := TRUE, PT := T#500MS);
    IF flash.Q THEN
        relay0 := NOT relay0;
        flash(IN := FALSE, PT := T#0MS);
    END_IF;
END_PROGRAM"#.into(),
        },
        SampleTemplate {
            id: "esp32-relay-chase".into(),
            name: "ESP32 Relay Chase".into(),
            description: "Chase pattern across 6 relays with RGB color cycling".into(),
            category: "ESP32 Hardware".into(),
            source: r#"PROGRAM ESP32_RelayChase
VAR
    buzzer : BOOL := FALSE;
    red : DINT := 0;
    green : DINT := 0;
    blue : DINT := 0;
    relay0 : BOOL := FALSE;
    relay1 : BOOL := FALSE;
    relay2 : BOOL := FALSE;
    relay3 : BOOL := FALSE;
    relay4 : BOOL := FALSE;
    relay5 : BOOL := FALSE;
    phase : DINT := 0;
    step_timer : TON;
END_VAR
    step_timer(IN := TRUE, PT := T#2S);
    IF step_timer.Q THEN
        step_timer(IN := FALSE, PT := T#0MS);
        phase := phase + 1;
        IF phase > 5 THEN
            phase := 0;
            buzzer := NOT buzzer;
        END_IF;
    END_IF;

    relay0 := FALSE; relay1 := FALSE; relay2 := FALSE;
    relay3 := FALSE; relay4 := FALSE; relay5 := FALSE;

    CASE phase OF
        0: relay0 := TRUE; red := 255; green := 0;   blue := 0;
        1: relay1 := TRUE; red := 0;   green := 255; blue := 0;
        2: relay2 := TRUE; red := 0;   green := 0;   blue := 255;
        3: relay3 := TRUE; red := 255; green := 255; blue := 0;
        4: relay4 := TRUE; red := 255; green := 0;   blue := 255;
        5: relay5 := TRUE; red := 0;   green := 255; blue := 255;
    END_CASE;
END_PROGRAM"#.into(),
        },
    ]
}

// ── Project Serialization (.vltrus files) ──

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ProjectSource {
    pub name: String,
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub is_active: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
pub struct ProjectAiSettings {
    pub provider: String,
    pub model: String,
    pub streaming: bool,
    pub ollama_endpoint: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ProjectFile {
    pub version: String,
    pub files: Vec<ProjectSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub io_config: Option<IoDriverConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub io_mappings: Vec<IoMapping>,
    pub scan_cycle_ms: u64,
    pub max_cycle_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_settings: Option<ProjectAiSettings>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub watch_list: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breakpoints: Vec<(String, Vec<u32>)>,
    /// Last target IP / SCADA URL (execution_mode is intentionally NOT
    /// persisted — live connections can't be restored).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scada_url: Option<String>,
    pub created_at: String,
}

#[tauri::command]
pub async fn save_project(path: String, project: ProjectFile) -> Result<(), String> {
    let json = serde_json::to_string_pretty(&project).map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::write(&path, &json).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn load_project(path: String) -> Result<ProjectFile, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let json = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read project file: {}", e))?;
        serde_json::from_str::<ProjectFile>(&json)
            .map_err(|e| format!("Failed to parse project file: {}", e))
    })
    .await
    .map_err(|e| e.to_string())?
}
