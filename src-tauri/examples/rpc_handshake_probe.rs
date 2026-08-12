//! Sends one read-only identity request to the physical Digitone II.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use midir::{Ignore, MidiInput, MidiOutput};
    use std::{sync::mpsc, time::Duration};

    let mut input = MidiInput::new("Digitone2Link handshake probe")?;
    input.ignore(Ignore::None);
    let input_ports = input.ports();
    let input_port = input_ports
        .iter()
        .find(|port| {
            input
                .port_name(port)
                .is_ok_and(|name| name == "Elektron Digitone II")
        })
        .ok_or("Physical Digitone MIDI input not found")?;
    println!("INPUT  {}", input.port_name(input_port)?);
    let (sender, receiver) = mpsc::channel::<Vec<u8>>();
    let _input_connection = input.connect(
        input_port,
        "digitone2link-handshake-probe-input",
        move |_timestamp, message, _| {
            let _ = sender.send(message.to_vec());
        },
        (),
    )?;

    let output = MidiOutput::new("Digitone2Link handshake probe")?;
    let output_ports = output.ports();
    let output_port = output_ports
        .iter()
        .find(|port| {
            output
                .port_name(port)
                .is_ok_and(|name| name == "Elektron Digitone II")
        })
        .ok_or("Physical Digitone MIDI output not found")?;
    println!("OUTPUT {}", output.port_name(output_port)?);
    let mut connection = output.connect(output_port, "digitone2link-handshake-probe-output")?;
    let request = [
        0xf0, 0x00, 0x20, 0x3c, 0x10, 0x00, 0x20, 0x00, 0x07, 0x00, 0x00, 0x01, 0xf7,
    ];
    println!(
        "SEND   {}",
        request
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    connection.send(&request)?;
    match receiver.recv_timeout(Duration::from_secs(5)) {
        Ok(response) => println!(
            "RECV   {}",
            response
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Err(_) => println!("RECV   timeout"),
    }
    Ok(())
}
