use eframe::egui;
use nix::{
    errno::Errno,
    fcntl::{fcntl, FcntlArg, OFlag},
    pty::{forkpty, ForkptyResult},
};

use std::{
    ffi::CStr,
    os::fd::{AsFd, AsRawFd, OwnedFd},
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

                // // For standardizing the shell prompts to `$`
                // std::env::remove_var("PROMPT_COMMAND");
                // std::env::set_var("PS1", "$ ");

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
    fd: OwnedFd,
    buf: Vec<u8>,
}

impl Termion {
    fn new(_cc: &eframe::CreationContext<'_>, fd: OwnedFd) -> Self {
        Termion {
            fd,
            buf: Vec::new(),
        }
    }
}

impl eframe::App for Termion {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut buf = vec![0u8; 4096];
        // let mut string;
        println!(":");
        match nix::unistd::read(self.fd.as_raw_fd(), &mut buf) {
            Ok(0) => {
                println!("EOF reached");
                return;
            }
            Ok(read_size) => {
                self.buf.extend_from_slice(&buf[0..read_size]);
            }
            Err(e) => {
                if e != Errno::EAGAIN {
                    println!("Read Failed due to: {}", e);
                } else {
                    println!("-");
                }
            }
        }

        let binding = self.buf.clone();
        let str_temp = std::str::from_utf8(&binding).unwrap();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Hello World! I am working on a new project");
            ui.input(|input_state| {
                for event in &input_state.events {
                    let egui::Event::Text(text) = event else {
                        continue;
                    };

                    let bytes = text.as_bytes();
                    let mut to_write: &[u8] = &bytes;

                    while to_write.len() > 0 {
                        let written = nix::unistd::write(self.fd.as_fd(), to_write).unwrap();
                        to_write = &to_write[written..];
                    }
                }
            });
            ui.label(str_temp);
        });
    }
}
