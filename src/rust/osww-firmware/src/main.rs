//! Binary entrypoint for the Winderoo firmware.

#[cfg(feature = "esp32")]
fn main() {
    if let Err(err) = winderoo_firmware::esp32::run() {
        panic!("ESP32 runtime error: {err}");
    }
}

#[cfg(not(feature = "esp32"))]
fn main() {
    eprintln!("winderoo-firmware built without the 'esp32' feature; no device runtime to execute.");
}
