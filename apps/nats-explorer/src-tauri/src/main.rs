#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    nats_explorer_lib::run();
}
