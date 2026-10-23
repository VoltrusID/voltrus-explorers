// Release builds on Windows: GUI subsystem, no console window behind the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    opcua_explorer_lib::run();
}
