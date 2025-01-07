use eframe::egui;
use nix::{
    errno::Errno,
    fcntl::{fcntl, FcntlArg, OFlag},
    pty::{forkpty, ForkptyResult},
};

use core::f32;
use std::{
    ffi::CStr, os::fd::{AsFd, AsRawFd, OwnedFd}, process::exit
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

pub struct Termion {
    fd: OwnedFd,
    buf: Vec<u8>,
    command_history: Vec<String>, // Store all commands TODO: Add delete button, add persistence
    current_command: String,      // Tracks current command pre enter press
    cursor_pos: (usize, usize),   // Window space and scroll back
    character_size: Option<(f32, f32)>,
    output_buf: OutputBuffer
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
            output_buf: OutputBuffer::new(),
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


fn accumulate_csi_buf(buf: &[u8]) -> Option<usize> {
    if buf.is_empty() {
        return  None;
    }

    let n = std::str::from_utf8(buf).expect("ASCII digits are expected to be pared  as utf8 to usize unless negative").parse().expect("Valid numbers should be able to be parsed into usize unless negative");
    return Some(n);
}

fn is_csi_terminator(b: u8) -> bool {
    match b {
        b'A' | b'B' | b'C' | 
        b'D' | b'E' | b'F' | 
        b'G' | b'H' | b'J' | 
        b'K' | b'S' | b'T' | 
        b'f' => true,
        _ => false,
        //aux ones are not supported
    }
}


enum TerminalOutput {
    SetCursorPos {
        x: usize,
        y: usize
    },
    Data(Vec<u8>)
}

#[derive(Eq, PartialEq)]
enum CsiParserState {
    N (Vec<u8>), 
    M(Vec<u8>),
    Finished(u8), // u8 Because there are different terminal values for the differnt code with same params, like `n;m H` and `n;m l`. u8 tracks the last value
    Invalid
}
struct CsiParser {
    state: CsiParserState,
    n: Option<usize>, // Generic word in CSI codes for the row
    m: Option<usize>, // Generic word in CSI codes for the col
}

impl CsiParser {
    fn new() -> CsiParser{
        CsiParser {
            state: CsiParserState::N(Vec::new()),
            n: None,
            m: None
        }
    }

    fn push(&mut self, b:u8) {
        // assert!(self.state != CsiParserState::Finished(_));
        if let CsiParserState::Finished(_) = &self.state {
            panic!("This should not happen");
        }

        if b == b'H' {
            self.state = CsiParserState::Finished(b'H');
            return;
        }

        match &mut self.state {
            CsiParserState::N(buf) => {
                if is_csi_terminator(b) {
                    self.state = CsiParserState::Finished(b);
                    return;
                }
                if b == b';' {
                    self.n = accumulate_csi_buf(buf);
                    self.state = CsiParserState::M(Vec::new());
                } else if b.is_ascii_digit() {
                    buf.push(b);
                } else {
                    let printable = char::from_u32(b.clone() as u32).unwrap();
                    panic!("Unexpected character in n: {b:x} {}", printable);
                }
            },
            CsiParserState::M(buf) => {
                if is_csi_terminator(b) {
                    self.m = accumulate_csi_buf(buf);
                    self.state = CsiParserState::Finished(b);
                } else if b.is_ascii_digit() {
                    buf.push(b);
                } else {
                    let printable = char::from_u32(b.clone() as u32).unwrap();
                    panic!("Unexpected character in m: {b:x} {}", printable);
                }                
            },
            CsiParserState::Finished(_) => {
                panic!("CsiParserState::Finished Should not be rechable")
            },
            CsiParserState::Invalid => {
                panic!("CsiParserState::Invalid Should not be rechable")
            },
        }
    }
}
enum AnsiBuilder {
    Empty,
    Escape,
    Csi(CsiParser),  // This is the control sequence introducer '[' and ']'
}
pub struct OutputBuffer{
    // buf: Vec<u8>, 
    current_state: AnsiBuilder
}

impl OutputBuffer {
    pub fn new() -> OutputBuffer{
        OutputBuffer{
            current_state: AnsiBuilder::Empty,
        }
    }
    fn push(&mut self, incoming: &[u8]) -> Vec<TerminalOutput>{
        let mut output = Vec::new();        
        let mut data_output = Vec::new();
        
        for b in incoming {
            
            println!("{} {b:x}", *b as char);

            match &mut self.current_state {
                AnsiBuilder::Empty => {
                    if *b == b'\x1b'{
                        // This is [ aka the control sequence introducer
                        self.current_state = AnsiBuilder::Escape;
                        continue;
                    } else {
                        data_output.push(*b);
                    }
                }, 
                AnsiBuilder::Escape => {
                    output.push(TerminalOutput::Data(std::mem::take(&mut data_output)));
                    // panic!("Unhandled escape sequence: {b:x}");
                    match b {
                        b'[' => {
                            self.current_state = AnsiBuilder::Csi(CsiParser::new());
                        }
                        _ => {
                            let printable = char::from_u32(*b as u32).unwrap();
                            panic!("Unhandled escape sequence: {b:x} {}", printable);
                        }
                    }
                },
                AnsiBuilder::Csi(parser) => {
                    parser.push(*b);
                    match &parser.state {
                        // CsiParserState::N(vec) => {},
                        // CsiParserState::M(vec) => {},
                        CsiParserState::Finished(b'H') => {
                            // Request to move the cursor position
                            // unwrap or 1 cause 1 is the default
                            output.push(TerminalOutput::SetCursorPos { x: parser.n.unwrap_or(1), y: parser.m.unwrap_or(1) });
                            self.current_state = AnsiBuilder::Empty;
                        },
                        _ => {
                            // Some other request
                            println!("Some other request/ state");
                        },
                    }
                }
            }
        }
        
        if !data_output.is_empty() {
            output.push(TerminalOutput::Data(std::mem::take(&mut data_output)));
        }

        output
    }
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
                let parsed = self.output_buf.push(incoming);
                for segment in parsed {
                    match segment {
                        // TerminalOutput::Ansi(_vec) => {                            
                        //     println!("To do");
                        // },
                        TerminalOutput::Data(_vec) => {
                            println!("not to do")
                        },
                        TerminalOutput::SetCursorPos { x, y } => {
                            panic!("need to update cursor position");
                        },
                    }
                }
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
        let cleaned_output: String = binding
            .iter()
            .filter(|&&c| c.is_ascii_graphic() || c.is_ascii_whitespace())
            .map(|&c| c as char)
            .collect();

        // println!("cleaned_output: {}", cleaned_output);
        // cleaned_output = cleaned_output.replace("[?2004h", "").replace("[?2004l", "");

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
                                        println!("Backspace pressed TODO: Handle it");
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
                    // println!("{} {}", x_offset, y_offset);
                    ctx.request_repaint(); // Explicitly request a repaint
                });
        });
    }
}
