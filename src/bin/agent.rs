#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(non_snake_case)] // Windows product executable name.

fn main() -> anyhow::Result<()> {
    straja_sql::worker::run(straja_sql::store::data_dir()?)
}
