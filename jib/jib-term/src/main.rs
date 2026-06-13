use std::{io::Write, path::PathBuf, time::Duration};

use cbfs_lib::CbContainerOptions;
use clap::Parser;
use jib_computer::{ComputerError, JibComputer};

#[derive(Default, Debug, Parser)]
#[command(version, about)]
struct Args {
    #[arg(
        short = 'e',
        long = "export-hd",
        help = "exports the hard disk image if provided to a file"
    )]
    export_hd: Option<PathBuf>,
}

fn main() -> Result<(), ComputerError> {
    let args = Args::parse();

    let mut computer = JibComputer::new()?;

    if let Some(dest) = args.export_hd {
        use cbfs_lib::{ContainerHeader, save_container};

        save_container(
            &ContainerHeader::new(CbContainerOptions {
                compressed: false,
                sparse: true,
            }),
            &cbfs_lib::FileSystem::read_bytes(&mut computer.get_disk_data()?.as_slice())?,
            &dest,
        )
        .unwrap();

        return Ok(());
    }

    computer.use_bootloader(true)?;
    computer.reset(None)?;
    computer.set_running_request(true);

    let stdin_channel = spawn_stdin_channel();

    loop {
        for _ in 0..1000 {
            match stdin_channel.try_recv() {
                Ok(input) => {
                    computer.set_serial_input(&input)?;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => panic!("stdin disconnected"),
            }
        }

        if computer.get_running() {
            for _ in 0..10000 {
                if !computer.step_cpu(None, true)? {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        } else if computer.get_running_requested() {
            computer.step_devices()?;
            std::thread::sleep(Duration::from_millis(100));
        } else {
            computer.set_running_request(true);
        }

        let chars = computer.get_serial_output_unknown();
        if !chars.is_empty() {
            let mut s = std::io::stdout();
            s.write(&chars.into_iter().map(|x| x as u8).collect::<Vec<_>>())
                .unwrap();
            s.flush().unwrap();
        }
    }
}

fn spawn_stdin_channel() -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        loop {
            let mut buffer = String::new();
            std::io::stdin().read_line(&mut buffer).unwrap();
            tx.send(buffer.trim().to_owned()).unwrap();
        }
    });
    rx
}
