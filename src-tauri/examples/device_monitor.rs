//! Development-only passive monitor for replies emitted by a connected Digitone II.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use midir::{Ignore, MidiInput};

    let mut input = MidiInput::new("Digitone Presets passive protocol monitor")?;
    input.ignore(Ignore::None);
    let ports = input.ports();
    let (port, name) = ports
        .iter()
        .filter_map(|port| input.port_name(port).ok().map(|name| (port, name)))
        .find(|(_, name)| name.to_lowercase().contains("digitone"))
        .ok_or("No Digitone MIDI input was found")?;
    println!("Passively monitoring: {name}");
    let _connection = input.connect(
        port,
        "digitone-presets-passive-monitor",
        |_stamp, message, _| {
            let hex = message
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            println!("DEVICE -> TRANSFER  {hex}");
        },
        (),
    )?;
    loop {
        std::thread::park();
    }
}
