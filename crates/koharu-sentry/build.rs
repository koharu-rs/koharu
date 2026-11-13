const COMMANDS: &[&str] = &["envelope", "breadcrumb", "scope"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
