//! Development tool: prints what the Windows audio engine grants a shared-mode capture client
//! for several requested buffer sizes on the input device whose name contains the first argument
//! (default "CABLE Output"). Opens and releases a probe client only; records nothing.
//! Run: cargo run -p wb-recorder --example buffer_probe -- "CABLE Output"

use cpal::traits::{DeviceTrait, HostTrait};

fn main() {
    let wanted = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "CABLE Output".into());
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else {
        eprintln!("no input devices");
        return;
    };
    for device in devices {
        let name = device.to_string();
        if !name.contains(&wanted) {
            continue;
        }
        let Ok(id) = device.id() else { continue };
        println!("device: {name}");
        println!("endpoint: {}", id.id());
        for ms in [100u32, 200, 300, 500, 700, 1000, 1500, 2000] {
            match wb_recorder::sys::probe_capture_buffer(id.id(), ms) {
                Some(g) => println!(
                    "requested {ms:>5} ms -> granted {:>8.2} ms ({} frames at {} Hz), engine period {:.2} ms",
                    g.granted_ms, g.granted_frames, g.rate, g.engine_period_ms
                ),
                None => println!("requested {ms:>5} ms -> refused or not probeable"),
            }
        }
    }
}
