//! Development probe for observing the discovery packets sent by Elektron Transfer.
//! It creates virtual CoreMIDI ports and never communicates with physical hardware.

#[cfg(unix)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use midir::os::unix::{VirtualInput, VirtualOutput};
    use midir::{Ignore, MidiInput, MidiOutput};

    let mut input = MidiInput::new("Digitone2Link protocol probe input")
        .map_err(|error| format!("create MIDI input client: {error}"))?;
    input.ignore(Ignore::None);
    let _incoming = input
        .create_virtual(
            "DP Protocol Probe IN",
            |_stamp, message, _| {
                let hex = message
                    .iter()
                    .map(|byte| format!("{byte:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                println!("TRANSFER -> DEVICE  {hex}");
            },
            (),
        )
        .map_err(|error| format!("create virtual destination: {error}"))?;

    let output = MidiOutput::new("Digitone2Link protocol probe output")
        .map_err(|error| format!("create MIDI output client: {error}"))?;
    let _outgoing = output
        .create_virtual("DP Protocol Probe OUT")
        .map_err(|error| format!("create virtual source: {error}"))?;
    println!("Virtual Digitone II ports are ready. Open Transfer and refresh devices.");
    loop {
        std::thread::park();
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("The development probe currently supports CoreMIDI/ALSA only.");
}
