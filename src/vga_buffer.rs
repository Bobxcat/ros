use core::fmt::Write;
use core::{cell::OnceCell, fmt, ops::DerefMut, ptr::NonNull};

use spin::{Lazy, Mutex};
use volatile::Volatile;
use x86_64::instructions::interrupts;
use x86_64::instructions::port::Port;

use crate::serial_println;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
struct ColorCode(u8);

impl ColorCode {
    fn new(foreground: Color, background: Color) -> ColorCode {
        ColorCode((background as u8) << 4 | (foreground as u8))
    }
}

impl Default for ColorCode {
    fn default() -> Self {
        Self::new(Color::White, Color::Black)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct ScreenChar {
    ascii_character: u8,
    color_code: ColorCode,
}

impl ScreenChar {
    /// A space with default coloring
    fn blank() -> Self {
        Self {
            ascii_character: b' ',
            color_code: ColorCode::default(),
        }
    }
    /// A null character with default coloring, used to denote the end of a line
    ///
    /// If you are not trying to denote the end of a line, consider using `blank`
    fn null() -> Self {
        Self {
            ascii_character: b'\0',
            color_code: ColorCode::default(),
        }
    }
}

const BUFFER_HEIGHT: usize = 25;
const BUFFER_WIDTH: usize = 80;

#[repr(transparent)]
#[derive(Debug, Clone)]
struct VgaBuffer {
    chars: [[Volatile<ScreenChar>; BUFFER_WIDTH]; BUFFER_HEIGHT],
}

pub struct VgaWriter {
    column_position: usize,
    color_code: ColorCode,
    buffer: &'static mut VgaBuffer,
}

impl VgaWriter {
    pub fn lock() -> impl DerefMut<Target = Self> {
        static VGA_WRITER: Lazy<Mutex<VgaWriter>> = Lazy::new(|| {
            let mut w = VgaWriter {
                column_position: 0,
                color_code: ColorCode::default(),
                buffer: unsafe { &mut *(0xb8000 as *mut VgaBuffer) },
            };
            w.clear();
            Mutex::new(w)
        });
        VGA_WRITER.lock()
    }
    pub fn set_colors(&mut self, foreground: Color, background: Color) {
        self.color_code = ColorCode::new(foreground, background);
    }
    pub fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
            // Backspace
            0x8 => self.backspace(),
            byte => {
                if self.column_position >= BUFFER_WIDTH {
                    self.new_line();
                }

                let row = BUFFER_HEIGHT - 1;
                let col = self.column_position;

                let color_code = self.color_code;
                self.buffer.chars[row][col].write(ScreenChar {
                    ascii_character: byte,
                    color_code,
                });
                self.column_position += 1;
            }
        }
        self.set_cursor_pos(BUFFER_HEIGHT - 1, self.column_position);
    }

    fn new_line(&mut self) {
        if self.column_position < BUFFER_WIDTH {
            self.buffer.chars[BUFFER_HEIGHT - 1][self.column_position].write(ScreenChar::null());
        }
        self.scroll(-1);
        self.column_position = 0;
    }

    /// Moves all rows by `offset`, clearing left behind space.
    /// Keep in mind that a negative offset moves the rows up
    ///
    /// Does not change the cursor in any way
    pub fn scroll(&mut self, offset: isize) {
        let src = self.buffer.clone();
        self.clear();
        for y in 0..BUFFER_HEIGHT {
            for x in 0..BUFFER_WIDTH {
                let origin_x = x;
                let Ok(origin_y) = usize::try_from(y as isize - offset) else {
                    continue;
                };
                let Some(src_row) = src.chars.get(origin_y) else {
                    continue;
                };
                self.buffer.chars[y][x].write(src_row[origin_x].read());
            }
        }
    }

    pub fn copy_row(&mut self, src: usize, dest: usize) {
        if src == dest {
            return;
        }

        for x in 0..BUFFER_WIDTH {
            let c = self.buffer.chars[src][x].read();
            self.buffer.chars[dest][x].write(c);
        }
    }

    pub fn clear(&mut self) {
        for y in 0..BUFFER_HEIGHT {
            self.clear_row(y);
        }
    }

    #[inline]
    pub fn clear_row(&mut self, row: usize) {
        for x in 0..BUFFER_WIDTH {
            self.clear_char(row, x);
        }
        self.set_char(row, 0, ScreenChar::null());
    }

    #[inline]
    pub fn clear_char(&mut self, row: usize, col: usize) {
        self.set_char(row, col, ScreenChar::blank())
    }

    #[inline]
    fn set_char(&mut self, row: usize, col: usize, c: ScreenChar) {
        if row >= BUFFER_HEIGHT || col >= BUFFER_WIDTH {
            return;
        }
        self.buffer.chars[row][col].write(c);
    }

    fn backspace(&mut self) {
        if self.column_position == 0 {
            self.scroll(1);
            self.column_position = self.buffer.chars[BUFFER_HEIGHT - 1]
                .iter()
                .position(|c| c.read().ascii_character == b'\0')
                // Don't set to the last position in order to keep consecutive backspaces working
                .unwrap_or(BUFFER_WIDTH);
        } else {
            self.buffer.chars[BUFFER_HEIGHT - 1][self.column_position - 1]
                .write(ScreenChar::blank());
            self.column_position -= 1;
        }
    }

    pub fn write_string(&mut self, s: &str) {
        for byte in s.bytes() {
            match byte {
                // printable ASCII byte or newline
                0x20..=0x7e | b'\n' | 0x08 => self.write_byte(byte),
                // not part of printable ASCII range
                _ => self.write_byte(0xfe),
            }
        }
    }

    /// `start` and `end` refer to the rows (scanlines) of the cursor
    pub fn enable_cursor(&mut self, start: u8, end: u8) {
        let mut port0 = Port::<u8>::new(0x3D4);
        let mut port1 = Port::<u8>::new(0x3D5);

        unsafe {
            port0.write(0x0A);
            let x = (port1.read() & 0xC0) | start;
            port1.write(x);

            port0.write(0x0B);
            let x = (port1.read() & 0xE0) | end;
            port1.write(x);
        }
    }

    pub fn disable_cursor(&mut self) {
        let mut port0 = Port::<u8>::new(0x3D4);
        let mut port1 = Port::<u8>::new(0x3D5);
        unsafe {
            port0.write(0x0A);
            port1.write(0x20);
        }
    }

    pub fn set_cursor_pos(&mut self, row: usize, col: usize) {
        // From https://wiki.osdev.org/Text_Mode_Cursor#Moving_the_Cursor_2
        let pos = (row * BUFFER_WIDTH + col) as u16;
        let mut port0 = Port::<u8>::new(0x3D4);
        let mut port1 = Port::<u8>::new(0x3D5);
        unsafe {
            port0.write(0x0f);
            port1.write((pos & 0xFF) as u8);
            port0.write(0x0e);
            port1.write(((pos >> 8) & 0xFF) as u8);
        }
    }
}

impl core::fmt::Write for VgaWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.write_string(s);
        Ok(())
    }
}

/// Prints to the VGA text buffer
#[macro_export]
macro_rules! vga_print {
    ($($arg:tt)*) => ($crate::vga_buffer::_print(format_args!($($arg)*)));
}

/// Prints to the VGA text buffer and appends a newline
#[macro_export]
macro_rules! vga_println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::vga_print!("{}\n", format_args!($($arg)*)));
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    interrupts::without_interrupts(|| {
        VgaWriter::lock().write_fmt(args).unwrap();
    })
}

#[test_case]
fn test_vga_println() {
    for i in 1..=100 {
        vga_println!("Line {i} of 100");
    }
}

#[test_case]
fn test_vga_println_output() {
    let s = "Some test string that fits on a single line";

    interrupts::without_interrupts(|| {
        // Keep the writer locked to avoid an interrupt deadlock
        let mut writer = VgaWriter::lock();
        writeln!(writer, "\n{}", s).expect("writeln failed");
        for (i, c) in s.chars().enumerate() {
            let screen_char = writer.buffer.chars[BUFFER_HEIGHT - 2][i].read();
            assert_eq!(char::from(screen_char.ascii_character), c);
        }
    });
}
