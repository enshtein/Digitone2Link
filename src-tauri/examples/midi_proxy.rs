//! Development-only transparent CoreMIDI proxy for documenting read-only Transfer RPC.

fn hex(message: &[u8]) -> String {
    message
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use midir::os::unix::{VirtualInput, VirtualOutput};
    use midir::{Ignore, MidiInput, MidiOutput};
    use std::sync::{Arc, Mutex};

    let physical_output = MidiOutput::new("DP RPC Proxy physical output")?;
    let output_ports = physical_output.ports();
    let output_port = output_ports
        .iter()
        .find(|port| {
            physical_output
                .port_name(port)
                .is_ok_and(|name| name.to_lowercase().contains("digitone"))
        })
        .ok_or("No physical Digitone MIDI output was found")?;
    let to_device = Arc::new(Mutex::new(
        physical_output.connect(output_port, "dp-rpc-proxy-to-device")?,
    ));

    let mut virtual_destination = MidiInput::new("DP RPC Proxy virtual destination")?;
    virtual_destination.ignore(Ignore::None);
    let destination_output = Arc::clone(&to_device);
    let _from_transfer = virtual_destination.create_virtual(
        "DP RPC Proxy IN",
        move |_stamp, message, _| {
            println!("REQUEST  {}", hex(message));
            if let Ok(mut output) = destination_output.lock() {
                let _ = output.send(message);
            }
        },
        (),
    )?;

    let virtual_source = MidiOutput::new("DP RPC Proxy virtual source")?;
    let to_transfer = Arc::new(Mutex::new(
        virtual_source.create_virtual("DP RPC Proxy OUT")?,
    ));

    let mut physical_input = MidiInput::new("DP RPC Proxy physical input")?;
    physical_input.ignore(Ignore::None);
    let input_ports = physical_input.ports();
    let input_port = input_ports
        .iter()
        .find(|port| {
            physical_input
                .port_name(port)
                .is_ok_and(|name| name.to_lowercase().contains("digitone"))
        })
        .ok_or("No physical Digitone MIDI input was found")?;
    let source_output = Arc::clone(&to_transfer);
    let _from_device = physical_input.connect(
        input_port,
        "dp-rpc-proxy-from-device",
        move |_stamp, message, _| {
            println!("RESPONSE {}", hex(message));
            if let Ok(mut output) = source_output.lock() {
                let _ = output.send(message);
            }
        },
        (),
    )?;

    println!("Proxy ready: Transfer IN = DP RPC Proxy OUT; OUT = DP RPC Proxy IN");
    loop {
        std::thread::park();
    }
}
