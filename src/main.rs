use eframe::egui;
use nix::{
    errno::Errno,
    fcntl::{fcntl, FcntlArg, OFlag},
    pty::{forkpty, ForkptyResult},
};

use core::f32;
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
                // File in non blocking mode to avoid freezing issue
                fcntl(master.as_raw_fd(), FcntlArg::F_SETFL(OFlag::O_NONBLOCK))
                    .expect("Failed to set non-blocking mode");
                Some(master) // Return the master file descriptor
            }
            ForkptyResult::Child => {
                println!("Child process. Proceeding to execute shell...");
                let shell_name = CStr::from_bytes_until_nul(b"/bin/bash\0")
                    .expect("Something went wrong in creating the shell_name");
                let args: [&CStr; 0] = [];

                // // For standardizing the shell prompts to `$`
                // // Also solves the issue of double enter on pressing one enter
                std::env::remove_var("PROMPT_COMMAND");
                std::env::set_var("PS1", "$");
                // std::env::set_var("PS1", "\\[\\e[?2004l\\]$ ");
                //
                // Disable bracketed paste mode
                std::env::set_var("TERM", "dumb");

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

struct Termion {
    fd: OwnedFd,
    buf: Vec<u8>,
    command_history: Vec<String>, // Store all commands TODO: Add delete button, add persistence
    current_command: String,      // Tracks current command pre enter press
    cursor_pos: (usize, usize),   // Window space and scroll back
    character_size: Option<(f32, f32)>,
}

impl Termion {
    fn new(cc: &eframe::CreationContext<'_>, fd: OwnedFd) -> Self {
        let mut font_id = None;
        cc.egui_ctx.style_mut(|style| {
            style.override_text_style = Some(egui::TextStyle::Monospace);
            font_id = Some(style.text_styles[&egui::TextStyle::Monospace].clone())
        });

        Termion {
            fd,
            buf: Vec::new(),
            command_history: Vec::new(),
            current_command: String::new(),
            cursor_pos: (0, 0),
            character_size: None,
        }
    }
}
fn get_char_size(cc: &egui::Context) -> (f32, f32) {
    let font_id = cc.style().text_styles[&egui::TextStyle::Monospace].clone();
    let (width, height) = cc.fonts(|fonts| {
        let layout = fonts.layout(
            "@".to_string(),
            font_id,
            egui::Color32::default(),
            f32::INFINITY,
        );
        (layout.mesh_bounds.width(), layout.mesh_bounds.height())
    });

    println!("Character dimentions are: {}, {}", width, height);

    return (width, height);
}

fn char_to_cursor_offset(
    character_pos: &(usize, usize),
    character_size: &(f32, f32),
    content: &[u8],
) -> (f32, f32) {
    let content_by_lines: Vec<&[u8]> = content.split(|b| *b == b'\n').collect();
    let num_lines = content_by_lines.len();
    // let last_line = content_by_lines.last().unwrap_or(&[0u8]);
    let x_offset = character_pos.0 as f32 * character_size.0;
    let y_offset = (character_pos.1 as i64 - num_lines as i64) as f32 * character_size.1;
    (x_offset, y_offset)
}

impl eframe::App for Termion {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.character_size.is_none() {
            self.character_size = Some(get_char_size(ctx));
            println!("self.character_size: {:?}", self.character_size);
        }

        let mut buf = vec![0u8; 4096];
        // println!(":");
        match nix::unistd::read(self.fd.as_raw_fd(), &mut buf) {
            Ok(0) => {
                println!("EOF reached");
                return;
            }
            Ok(read_size) => {
                let incoming = &buf[0..read_size];
                for c in incoming {
                    match c {
                        b'\n' => self.cursor_pos = (0, 1 + self.cursor_pos.1),
                        _ => self.cursor_pos = (1 + self.cursor_pos.0, self.cursor_pos.1),
                    }
                }
                self.buf.extend_from_slice(incoming);
            }
            Err(e) => {
                if e != Errno::EAGAIN {
                    println!("Read Failed due to: {}", e);
                    // exit(1); // Kill the emulator if there is error;
                } else {
                    // println!("-");
                }
            }
        }

        // Side panel remains the same...
        egui::SidePanel::right("history_panel")
            .min_width(100.0)
            .show(ctx, |ui| {
                ui.heading("Command History");
                ui.separator();
                for cmd in &self.command_history {
                    if ui.button(cmd).clicked() {
                        println!("Clicked:: {}", cmd);
                        self.current_command.clear();
                        let cmd_with_newline = format!("{}\n", cmd);
                        let bytes = cmd_with_newline.as_bytes();
                        let mut to_write: &[u8] = &bytes;
                        while to_write.len() > 0 {
                            match nix::unistd::write(self.fd.as_fd(), to_write) {
                                Ok(written) => to_write = &to_write[written..],
                                Err(e) => {
                                    println!("Failed to write command to terminal: {}", e);
                                    break;
                                }
                            }
                        }
                        println!("Executed command from sidepanel: {}", cmd);
                    }
                }
            });

        let binding = self.buf.clone();
        let mut cleaned_output: String = binding
            .iter()
            .filter(|&&c| c.is_ascii_graphic() || c.is_ascii_whitespace())
            .map(|&c| c as char)
            .collect();

        cleaned_output = cleaned_output.replace("[?2004h", "").replace("[?2004l", "");

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::both()
                .auto_shrink([false; 2]) // Prevent shrinking; ensures resizing works
                .stick_to_bottom(true) // For large commands, helps keep ip part in focus
                .show(ui, |ui| {
                    ui.input(|input_state| {
                        for event in &input_state.events {
                            let text = match event {
                                egui::Event::Text(text) => {
                                    self.current_command.push_str(text);
                                    text
                                }
                                egui::Event::Key { key, pressed, .. } => match key {
                                    egui::Key::Enter => {
                                        if !self.current_command.trim().is_empty() {
                                            self.command_history.push(self.current_command.clone());
                                        }
                                        self.current_command.clear();
                                        "\n"
                                    }
                                    egui::Key::Backspace => {
                                        println!("Hello world");
                                        if *pressed && !self.current_command.is_empty() {
                                            self.current_command.pop();
                                            let backspace_char = b'\x08'; // ASCII backspace character
                                            nix::unistd::write(self.fd.as_fd(), &[backspace_char])
                                                .unwrap();
                                            ""
                                            // "\x08" // ASCII backspace character, TODO: Get ansi escape codes to work, the backspace is working but not reflected in the UI
                                            // "\x7F" // Delete character (DEL)
                                        } else {
                                            ""
                                        }
                                    }
                                    _ => "",
                                },
                                _ => "",
                            };

                            // let temp_text = &text.replace("[?2004h", "").replace("[?2004l", "");
                            let temp_text = &text;
                            let bytes = temp_text.as_bytes();

                            let mut to_write: &[u8] = &bytes;
                            while to_write.len() > 0 {
                                let written =
                                    nix::unistd::write(self.fd.as_fd(), to_write).unwrap();
                                to_write = &to_write[written..];
                            }
                        }
                    });
                    let response = ui.label(cleaned_output);

                    let left = response.rect.left();
                    let bottom = response.rect.bottom();

                    let painter = ui.painter();
                    let character_size = self.character_size.as_ref().unwrap();
                    let (x_offset, y_offset) =
                        char_to_cursor_offset(&self.cursor_pos, character_size, &self.buf);

                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(left + x_offset, bottom + y_offset),
                            egui::vec2(character_size.0, character_size.1),
                        ),
                        0.0,
                        egui::Color32::GREEN,
                    );
                    println!("{} {}", x_offset, y_offset);
                    ctx.request_repaint(); // Explicitly request a repaint
                });
        });
    }
}
