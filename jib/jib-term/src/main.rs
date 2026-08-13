use std::{
    fs::File,
    io::Write,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use cbfs_lib::{CbContainerOptions, ContainerHeader, read_container, save_container};
use clap::Parser;
use jib_computer::{ComputerError, JibComputer};

#[derive(Default, Debug, Parser)]
#[command(version, about)]
struct Args {
    #[arg(
        short = 'e',
        long = "export-hd",
        help = "exports the hard disk image if provided to a file. If provided with the disk option, this will overwrite the save location"
    )]
    export_hd: Option<PathBuf>,
    #[arg(
        short = 'd',
        long = "disk",
        help = "loads the provided HD image if provided, and will save if needed"
    )]
    hd_image: Option<PathBuf>,
    #[arg(
        short = 'r',
        long = "read-only",
        help = "marks the input file as read-only",
        default_value_t = false
    )]
    read_only: bool,
    #[arg(
        short = 'n',
        long = "no-boot",
        help = "will only run compiling/initialization phase, but will not boot or run the system"
    )]
    no_boot: bool,
}

fn main() -> Result<(), ComputerError> {
    let args = Args::parse();
    let mut computer = JibComputer::new()?;

    let (save_hdr, hd) = if let Some(hd_file) = args.hd_image.as_ref() {
        let mut file = File::open(hd_file).expect("unable to open HD file");
        read_container(&mut file)?
    } else {
        (
            ContainerHeader::new(CbContainerOptions {
                sparse: true,
                compressed: false,
            }),
            computer.get_os_image().create_hard_drive()?,
        )
    };

    computer.set_disk_filesystem(hd)?;
    computer.use_bootloader(true)?;
    computer.reset(None)?;

    if !args.no_boot {
        computer.set_running_request(true);
        let run_loop = Arc::new(AtomicBool::new(true));

        // Create the thread handler
        let r1 = run_loop.clone();
        ctrlc::set_handler(move || {
            r1.store(false, std::sync::atomic::Ordering::SeqCst);
        })
        .expect("unable to set ctrl-c handler");

        // Create the channel
        let (tx, stdin_channel) = std::sync::mpsc::channel::<String>();
        let r2 = run_loop.clone();
        std::thread::spawn(move || {
            while r2.load(std::sync::atomic::Ordering::SeqCst) {
                let mut buffer = String::new();
                std::io::stdin().read_line(&mut buffer).unwrap();
                tx.send(buffer.trim().to_owned()).unwrap();
            }
        });

        // Run the main loop
        while run_loop.load(std::sync::atomic::Ordering::SeqCst) {
            for _ in 0..1000 {
                match stdin_channel.try_recv() {
                    Ok(input) => {
                        computer.set_serial_input_unknown(&input)?;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        panic!("stdin disconnected")
                    }
                }
            }

            if computer.get_running() {
                for _ in 0..10000 {
                    if !computer.step_cpu(
                        None,
                        Some(jib_computer::StopMode {
                            cancel_run: true,
                            debug: false,
                        }),
                    )? {
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
                s.write_all(&chars.into_iter().map(|x| x as u8).collect::<Vec<_>>())
                    .unwrap();
                s.flush().unwrap();
            }
        }
    }

    if !args.read_only
        && let Some(save_path) = args.export_hd.or(args.hd_image)
    {
        save_container(
            &save_hdr,
            &cbfs_lib::FileSystem::read_bytes(&mut computer.get_disk_data()?.as_slice())?,
            &save_path,
        )?;
    }

    Ok(())
}
