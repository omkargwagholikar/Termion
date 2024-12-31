use eframe::egui;
use nix::{
    fcntl::{fcntl, FcntlArg, OFlag},
    pty::{forkpty, ForkptyResult},
};

use std::{
    ffi::CStr,
    fs::File,
    io::Read,
    os::fd::{AsRawFd, OwnedFd},
    process::exit,
};

fn main() {
    let fd: Option<OwnedFd> = unsafe {
        let res = forkpty(None, None).unwrap();
        match res {
            ForkptyResult::Parent { child, master } => {
                println!("Parent process. Child PID: {} Master FD: Some_value", child);
                fcntl(master.as_raw_fd(), FcntlArg::F_SETFL(OFlag::O_NONBLOCK))
                    .expect("Failed to set non-blocking mode");
                Some(master) // Return the master file descriptor
            }
            ForkptyResult::Child => {
                println!("Child process. Proceeding to execute shell...");
                let shell_name = CStr::from_bytes_until_nul(b"sh\0")
                    .expect("Something went wrong in creating the shell_name");
                let args: [&CStr; 0] = [];
                // [CStr::from_bytes_until_nul(b"sh\0").expect("Problem in setting the args")];
                std::env::remove_var("PROMPT_COMMAND");
                std::env::set_var("PS1", "$ ");
                nix::unistd::execvp(shell_name, &args).unwrap();

                exit(1);
            }
        }
    };

    if let Some(fd) = fd {
        println!("Fd read was successful");
        let native_options = eframe::NativeOptions::default();
        let _ = eframe::run_native(
            "Termion",
            native_options,
            Box::new(move |cc| Ok(Box::new(Termion::new(cc, fd)))),
        );
        println!("Completed");
    } else {
        println!("Fd read was unsuccessful");
    }
}

// #[derive(Default)]
struct Termion {
    fd: File,
    buf: Vec<u8>,
}

impl Termion {
    fn new(_cc: &eframe::CreationContext<'_>, fd: OwnedFd) -> Self {
        Termion {
            buf: Vec::new(),
            fd: fd.into(),
        }
    }
}

impl eframe::App for Termion {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut buf = vec![0u8; 4096];
        // let mut string;
        println!(":");
        match self.fd.read(&mut buf) {
            Ok(0) => {
                println!("EOF reached");
                return;
            }
            Ok(read_size) => {
                self.buf.extend_from_slice(&buf[0..read_size]);
            }
            Err(e) => {
                if e.kind() != std::io::ErrorKind::WouldBlock {
                    println!("Read Failed due to: {}", e);
                } else {
                    println!("-");
                }
            }
        }

        let str_temp = std::str::from_utf8(&self.buf).unwrap();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Hello World! I am working on a new project");
            ui.label(str_temp);
        });
    }
}
