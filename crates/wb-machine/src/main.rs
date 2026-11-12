//! `wb-machine`: prints what this process may really use, as JSON. Run it under `wb-emulate` to see
//! the 4 processors of `emu-low` and the 11 GB job limit where `std` and `GlobalMemoryStatusEx`
//! still report the whole Legion. Development tool; not bundled.

fn main() {
    let machine = wb_machine::probe();
    match serde_json::to_string_pretty(&machine) {
        Ok(text) => println!("{text}"),
        Err(e) => {
            eprintln!("wb-machine: {e}");
            std::process::exit(1);
        }
    }
}
