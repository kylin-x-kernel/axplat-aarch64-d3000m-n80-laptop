//! VGA Console with scrolling, configurable font size, and log buffer.

#![allow(dead_code)]

use font8x8::{UnicodeFonts, BASIC_FONTS};
use kspin::SpinNoIrq;
use lazyinit::LazyInit;

/// Screen dimensions
const SCREEN_WIDTH: usize = 1920;
const SCREEN_HEIGHT: usize = 1200;

/// Base font dimensions (8x8 font)
const BASE_FONT_WIDTH: usize = 8;
const BASE_FONT_HEIGHT: usize = 8;

/// Default font scale (1 = 8x8, 2 = 16x16, etc.)
const DEFAULT_FONT_SCALE: usize = 2;

/// Log buffer size (number of characters to cache)
const LOG_BUFFER_SIZE: usize = 64 * 1024; // 64KB buffer

/// VGA framebuffer base address
const VGA_BASE_ADDR: usize = 0xffff_0000_ecd2_0000;

/// Default foreground color (white)
const FG_COLOR: u32 = 0xFFFFFF;
/// Default background color (black)
const BG_COLOR: u32 = 0x000000;

/// ANSI color codes to RGB color mapping
const ANSI_COLORS: [u32; 16] = [
    0x000000, // 0: Black (30, 40)
    0xCC0000, // 1: Red (31, 41)
    0x00CC00, // 2: Green (32, 42)
    0xCCCC00, // 3: Yellow (33, 43)
    0x0000CC, // 4: Blue (34, 44)
    0xCC00CC, // 5: Magenta (35, 45)
    0x00CCCC, // 6: Cyan (36, 46)
    0xCCCCCC, // 7: White (37, 47)
    0x666666, // 8: Bright Black (90, 100)
    0xFF0000, // 9: Bright Red (91, 101)
    0x00FF00, // 10: Bright Green (92, 102)
    0xFFFF00, // 11: Bright Yellow (93, 103)
    0x0000FF, // 12: Bright Blue (94, 104)
    0xFF00FF, // 13: Bright Magenta (95, 105)
    0x00FFFF, // 14: Bright Cyan (96, 106)
    0xFFFFFF, // 15: Bright White (97, 107)
];

/// Convert ANSI color code to RGB color
fn ansi_to_rgb(code: u8) -> Option<u32> {
    match code {
        30..=37 => Some(ANSI_COLORS[(code - 30) as usize]),
        40..=47 => Some(ANSI_COLORS[(code - 40) as usize]),
        90..=97 => Some(ANSI_COLORS[(code - 90 + 8) as usize]),
        100..=107 => Some(ANSI_COLORS[(code - 100 + 8) as usize]),
        _ => None,
    }
}

/// ANSI escape sequence parser state
#[derive(Clone, Copy, PartialEq)]
enum AnsiState {
    Normal,
    Escape,      // After ESC (\x1B)
    Csi,         // After ESC [
}

static VGA: LazyInit<SpinNoIrq<VgaConsole>> = LazyInit::new();

/// Circular buffer (FIFO) for caching log history
pub struct LogBuffer {
    buffer: [u8; LOG_BUFFER_SIZE],
    head: usize,  // Write position
    tail: usize,  // Read position
    len: usize,   // Current number of elements
}

impl LogBuffer {
    /// Creates a new empty log buffer
    pub const fn new() -> Self {
        Self {
            buffer: [0u8; LOG_BUFFER_SIZE],
            head: 0,
            tail: 0,
            len: 0,
        }
    }

    /// Pushes a byte to the buffer, overwriting oldest data if full
    pub fn push(&mut self, byte: u8) {
        self.buffer[self.head] = byte;
        self.head = (self.head + 1) % LOG_BUFFER_SIZE;
        
        if self.len == LOG_BUFFER_SIZE {
            // Buffer is full, move tail forward (overwrite oldest)
            self.tail = (self.tail + 1) % LOG_BUFFER_SIZE;
        } else {
            self.len += 1;
        }
    }

    /// Pushes multiple bytes to the buffer
    pub fn push_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.push(b);
        }
    }

    /// Returns the number of bytes in the buffer
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Iterates over all bytes in the buffer (oldest to newest)
    pub fn iter(&self) -> LogBufferIter {
        LogBufferIter {
            buffer: &self.buffer,
            pos: self.tail,
            remaining: self.len,
        }
    }
}

/// Iterator for LogBuffer
pub struct LogBufferIter<'a> {
    buffer: &'a [u8; LOG_BUFFER_SIZE],
    pos: usize,
    remaining: usize,
}

impl<'a> Iterator for LogBufferIter<'a> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let byte = self.buffer[self.pos];
        self.pos = (self.pos + 1) % LOG_BUFFER_SIZE;
        self.remaining -= 1;
        Some(byte)
    }
}

/// VGA Console structure with configurable font size and log buffer
pub struct VgaConsole {
    base_addr: usize,
    cursor_x: usize,
    cursor_y: usize,
    font_scale: usize,
    max_cols: usize,
    max_rows: usize,
    fg_color: u32,
    bg_color: u32,
    default_fg_color: u32,
    default_bg_color: u32,
    log_buffer: LogBuffer,
    // ANSI escape sequence parser state
    ansi_state: AnsiState,
    ansi_param: u8,
}

impl VgaConsole {
    /// Creates a new VGA console with specified base address and font scale
    pub fn new(base_addr: usize, font_scale: usize) -> Self {
        let scale = if font_scale == 0 { 1 } else { font_scale };
        let font_width = BASE_FONT_WIDTH * scale;
        let font_height = BASE_FONT_HEIGHT * scale;
        
        Self {
            base_addr,
            cursor_x: 0,
            cursor_y: 0,
            font_scale: scale,
            max_cols: SCREEN_WIDTH / font_width,
            max_rows: SCREEN_HEIGHT / font_height,
            fg_color: FG_COLOR,
            bg_color: BG_COLOR,
            default_fg_color: FG_COLOR,
            default_bg_color: BG_COLOR,
            log_buffer: LogBuffer::new(),
            ansi_state: AnsiState::Normal,
            ansi_param: 0,
        }
    }

    /// Returns the current font width in pixels
    fn font_width(&self) -> usize {
        BASE_FONT_WIDTH * self.font_scale
    }

    /// Returns the current font height in pixels
    fn font_height(&self) -> usize {
        BASE_FONT_HEIGHT * self.font_scale
    }

    /// Draws a single pixel at (x, y) with the specified color
    fn draw_pixel(&self, x: usize, y: usize, color: u32) {
        if x >= SCREEN_WIDTH || y >= SCREEN_HEIGHT {
            return;
        }
        unsafe {
            let offset = y * SCREEN_WIDTH + x;
            core::ptr::write_volatile((self.base_addr as *mut u32).add(offset), color);
        }
    }

    /// Draws a character at (x, y) with foreground and background colors
    fn draw_char(&mut self, ch: char, x: usize, y: usize, fg_color: u32, bg_color: u32) {
        let glyph = BASIC_FONTS.get(ch).unwrap_or(BASIC_FONTS.get('?').unwrap());
        
        for (row, byte) in glyph.iter().enumerate() {
            for col in 0..8 {
                let is_set = (byte & (1 << col)) != 0;
                let color = if is_set { fg_color } else { bg_color };
                
                // Draw scaled pixel (font_scale x font_scale block)
                for dy in 0..self.font_scale {
                    for dx in 0..self.font_scale {
                        self.draw_pixel(
                            x + col * self.font_scale + dx,
                            y + row * self.font_scale + dy,
                            color,
                        );
                    }
                }
            }
        }
    }

    /// Scrolls the screen up by one line
    fn scroll_up(&mut self) {
        let font_height = self.font_height();
        
        unsafe {
            let ptr = self.base_addr as *mut u32;
            let row_pixels = font_height * SCREEN_WIDTH;
            let total_pixels = SCREEN_HEIGHT * SCREEN_WIDTH;
            let move_pixels = total_pixels - row_pixels;
            
            // Move all content up by one line
            core::ptr::copy(ptr.add(row_pixels), ptr, move_pixels);
            
            // Clear the bottom line
            let bottom_ptr = ptr.add(move_pixels);
            for i in 0..row_pixels {
                core::ptr::write_volatile(bottom_ptr.add(i), self.bg_color);
            }
        }
    }

    /// Clears the entire screen
    pub fn clear(&mut self) {
        unsafe {
            let ptr = self.base_addr as *mut u32;
            let total_pixels = SCREEN_WIDTH * SCREEN_HEIGHT;
            for i in 0..total_pixels {
                core::ptr::write_volatile(ptr.add(i), self.bg_color);
            }
        }
        self.cursor_x = 0;
        self.cursor_y = 0;
    }

    /// Process ANSI SGR (Select Graphic Rendition) parameter
    fn process_ansi_sgr(&mut self, param: u8) {
        match param {
            0 => {
                // Reset to default
                self.fg_color = self.default_fg_color;
                self.bg_color = self.default_bg_color;
            }
            1 => {
                // Bold - we can make the color brighter
                // For simplicity, just keep current color
            }
            30..=37 | 90..=97 => {
                // Foreground color
                if let Some(color) = ansi_to_rgb(param) {
                    self.fg_color = color;
                }
            }
            40..=47 | 100..=107 => {
                // Background color
                if let Some(color) = ansi_to_rgb(param) {
                    self.bg_color = color;
                }
            }
            39 => {
                // Default foreground color
                self.fg_color = self.default_fg_color;
            }
            49 => {
                // Default background color
                self.bg_color = self.default_bg_color;
            }
            _ => {
                // Unsupported SGR parameter, ignore
            }
        }
    }

    /// Writes a single byte to the console with ANSI escape sequence support
    pub fn write_byte(&mut self, byte: u8) {
        // Cache to log buffer
        self.log_buffer.push(byte);
        
        match self.ansi_state {
            AnsiState::Normal => {
                match byte {
                    0x1B => {
                        // ESC character - start escape sequence
                        self.ansi_state = AnsiState::Escape;
                    }
                    b'\n' => {
                        self.new_line();
                    }
                    b'\r' => {
                        self.cursor_x = 0;
                    }
                    b'\t' => {
                        // Handle tab as 4 spaces
                        let spaces = 4 - (self.cursor_x % 4);
                        for _ in 0..spaces {
                            self.write_visible_char(b' ');
                        }
                    }
                    _ => {
                        self.write_visible_char(byte);
                    }
                }
            }
            AnsiState::Escape => {
                match byte {
                    b'[' => {
                        // CSI (Control Sequence Introducer)
                        self.ansi_state = AnsiState::Csi;
                        self.ansi_param = 0;
                    }
                    _ => {
                        // Unknown escape sequence, return to normal
                        self.ansi_state = AnsiState::Normal;
                    }
                }
            }
            AnsiState::Csi => {
                match byte {
                    b'0'..=b'9' => {
                        // Accumulate numeric parameter
                        self.ansi_param = self.ansi_param.saturating_mul(10).saturating_add(byte - b'0');
                    }
                    b';' => {
                        // Parameter separator - process current parameter and continue
                        self.process_ansi_sgr(self.ansi_param);
                        self.ansi_param = 0;
                    }
                    b'm' => {
                        // SGR (Select Graphic Rendition) - end of sequence
                        self.process_ansi_sgr(self.ansi_param);
                        self.ansi_state = AnsiState::Normal;
                        self.ansi_param = 0;
                    }
                    _ => {
                        // Unknown CSI sequence, return to normal
                        self.ansi_state = AnsiState::Normal;
                        self.ansi_param = 0;
                    }
                }
            }
        }
    }

    /// Writes a visible character to the screen
    fn write_visible_char(&mut self, byte: u8) {
        if self.cursor_x >= self.max_cols {
            self.new_line();
        }
        
        let ch = byte as char;
        let x = self.cursor_x * self.font_width();
        let y = self.cursor_y * self.font_height();
        self.draw_char(ch, x, y, self.fg_color, self.bg_color);
        self.cursor_x += 1;
    }

    /// Moves to a new line, scrolling if necessary
    fn new_line(&mut self) {
        self.cursor_x = 0;
        self.cursor_y += 1;
        if self.cursor_y >= self.max_rows {
            self.scroll_up();
            self.cursor_y = self.max_rows - 1;
        }
    }

    /// Writes a slice of bytes to the console
    pub fn write_bytes(&mut self, s: &[u8]) {
        for &b in s {
            self.write_byte(b);
        }
    }

    /// Sets the font scale (1 = 8x8, 2 = 16x16, etc.)
    pub fn set_font_scale(&mut self, scale: usize) {
        let scale = if scale == 0 { 1 } else { scale };
        self.font_scale = scale;
        self.max_cols = SCREEN_WIDTH / self.font_width();
        self.max_rows = SCREEN_HEIGHT / self.font_height();
        
        // Adjust cursor if it's now out of bounds
        if self.cursor_x >= self.max_cols {
            self.cursor_x = self.max_cols - 1;
        }
        if self.cursor_y >= self.max_rows {
            self.cursor_y = self.max_rows - 1;
        }
    }

    /// Sets the foreground color
    pub fn set_fg_color(&mut self, color: u32) {
        self.fg_color = color;
    }

    /// Sets the background color
    pub fn set_bg_color(&mut self, color: u32) {
        self.bg_color = color;
    }

    /// Returns the number of cached log bytes
    pub fn log_buffer_len(&self) -> usize {
        self.log_buffer.len()
    }

    /// Redraws the screen from the log buffer (useful after font size change)
    /// This properly handles ANSI escape sequences
    pub fn redraw_from_log(&mut self) {
        // Reset screen and colors
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.fg_color = self.default_fg_color;
        self.bg_color = self.default_bg_color;
        self.ansi_state = AnsiState::Normal;
        self.ansi_param = 0;
        
        // Clear screen
        unsafe {
            let ptr = self.base_addr as *mut u32;
            let total_pixels = SCREEN_WIDTH * SCREEN_HEIGHT;
            for i in 0..total_pixels {
                core::ptr::write_volatile(ptr.add(i), self.default_bg_color);
            }
        }
        
        // Collect log buffer content
        let bytes: alloc::vec::Vec<u8> = self.log_buffer.iter().collect();
        
        // Replay all bytes with ANSI support but without re-buffering
        for &b in &bytes {
            match self.ansi_state {
                AnsiState::Normal => {
                    match b {
                        0x1B => {
                            self.ansi_state = AnsiState::Escape;
                        }
                        b'\n' => {
                            self.new_line();
                        }
                        b'\r' => {
                            self.cursor_x = 0;
                        }
                        b'\t' => {
                            let spaces = 4 - (self.cursor_x % 4);
                            for _ in 0..spaces {
                                self.write_visible_char(b' ');
                            }
                        }
                        _ => {
                            self.write_visible_char(b);
                        }
                    }
                }
                AnsiState::Escape => {
                    match b {
                        b'[' => {
                            self.ansi_state = AnsiState::Csi;
                            self.ansi_param = 0;
                        }
                        _ => {
                            self.ansi_state = AnsiState::Normal;
                        }
                    }
                }
                AnsiState::Csi => {
                    match b {
                        b'0'..=b'9' => {
                            self.ansi_param = self.ansi_param.saturating_mul(10).saturating_add(b - b'0');
                        }
                        b';' => {
                            self.process_ansi_sgr(self.ansi_param);
                            self.ansi_param = 0;
                        }
                        b'm' => {
                            self.process_ansi_sgr(self.ansi_param);
                            self.ansi_state = AnsiState::Normal;
                            self.ansi_param = 0;
                        }
                        _ => {
                            self.ansi_state = AnsiState::Normal;
                            self.ansi_param = 0;
                        }
                    }
                }
            }
        }
    }
}

/// Embedded logo PNG data
const LOGO_PNG: &[u8] = include_bytes!("../tools/arceos.png");

/// Display the logo image centered on a white background
fn display_logo(base_addr: usize) {
    // Decode PNG image
    let header = match minipng::decode_png_header(LOGO_PNG) {
        Ok(h) => h,
        Err(_) => return,
    };

    let mut buffer = alloc::vec![0u8; header.required_bytes()];
    let image = match minipng::decode_png(LOGO_PNG, &mut buffer) {
        Ok(img) => img,
        Err(_) => return,
    };

    let img_width = image.width() as usize;
    let img_height = image.height() as usize;
    let pixels = image.pixels();

    // Fill screen with white background
    unsafe {
        let ptr = base_addr as *mut u32;
        let total_pixels = SCREEN_WIDTH * SCREEN_HEIGHT;
        for i in 0..total_pixels {
            core::ptr::write_volatile(ptr.add(i), 0xFFFFFF); // White
        }
    }

    // Calculate centered position
    let x_offset = (SCREEN_WIDTH.saturating_sub(img_width)) / 2;
    let y_offset = (SCREEN_HEIGHT.saturating_sub(img_height)) / 2;

    // Draw the logo
    unsafe {
        let ptr = base_addr as *mut u32;
        
        match image.color_type() {
            minipng::ColorType::Rgba => {
                // RGBA: 4 bytes per pixel
                for y in 0..img_height {
                    for x in 0..img_width {
                        let screen_x = x_offset + x;
                        let screen_y = y_offset + y;
                        if screen_x < SCREEN_WIDTH && screen_y < SCREEN_HEIGHT {
                            let idx = (y * img_width + x) * 4;
                            if idx + 3 < pixels.len() {
                                let r = pixels[idx] as u32;
                                let g = pixels[idx + 1] as u32;
                                let b = pixels[idx + 2] as u32;
                                let a = pixels[idx + 3] as u32;
                                
                                // Alpha blending with white background
                                let bg_r = 255u32;
                                let bg_g = 255u32;
                                let bg_b = 255u32;
                                
                                let final_r = (r * a + bg_r * (255 - a)) / 255;
                                let final_g = (g * a + bg_g * (255 - a)) / 255;
                                let final_b = (b * a + bg_b * (255 - a)) / 255;
                                
                                let color = (final_r << 16) | (final_g << 8) | final_b;
                                let offset = screen_y * SCREEN_WIDTH + screen_x;
                                core::ptr::write_volatile(ptr.add(offset), color);
                            }
                        }
                    }
                }
            }
            minipng::ColorType::Rgb => {
                // RGB: 3 bytes per pixel
                for y in 0..img_height {
                    for x in 0..img_width {
                        let screen_x = x_offset + x;
                        let screen_y = y_offset + y;
                        if screen_x < SCREEN_WIDTH && screen_y < SCREEN_HEIGHT {
                            let idx = (y * img_width + x) * 3;
                            if idx + 2 < pixels.len() {
                                let r = pixels[idx] as u32;
                                let g = pixels[idx + 1] as u32;
                                let b = pixels[idx + 2] as u32;
                                let color = (r << 16) | (g << 8) | b;
                                let offset = screen_y * SCREEN_WIDTH + screen_x;
                                core::ptr::write_volatile(ptr.add(offset), color);
                            }
                        }
                    }
                }
            }
            minipng::ColorType::GrayAlpha => {
                // Grayscale + Alpha: 2 bytes per pixel
                for y in 0..img_height {
                    for x in 0..img_width {
                        let screen_x = x_offset + x;
                        let screen_y = y_offset + y;
                        if screen_x < SCREEN_WIDTH && screen_y < SCREEN_HEIGHT {
                            let idx = (y * img_width + x) * 2;
                            if idx + 1 < pixels.len() {
                                let gray = pixels[idx] as u32;
                                let a = pixels[idx + 1] as u32;
                                
                                // Alpha blending with white background
                                let final_gray = (gray * a + 255 * (255 - a)) / 255;
                                let color = (final_gray << 16) | (final_gray << 8) | final_gray;
                                let offset = screen_y * SCREEN_WIDTH + screen_x;
                                core::ptr::write_volatile(ptr.add(offset), color);
                            }
                        }
                    }
                }
            }
            _ => {
                // Unsupported color type, skip
            }
        }
    }
}


/// Simple busy-wait delay (approximately 1 second)
fn delay_1s() {
    // Use ARM generic timer for delay
    // Read CNTFRQ_EL0 to get timer frequency
    let freq: u64;
    let start: u64;
    unsafe {
        core::arch::asm!("mrs {}, cntfrq_el0", out(reg) freq);
        core::arch::asm!("mrs {}, cntpct_el0", out(reg) start);
    }
    
    // Wait for 1 second
    let target = start + freq;
    loop {
        let current: u64;
        unsafe {
            core::arch::asm!("mrs {}, cntpct_el0", out(reg) current);
        }
        if current >= target {
            break;
        }
    }
}

/// Initializes the VGA console with specified font scale and base address
/// 
/// This should be called in `init_later` after memory management is ready.
/// 
/// # Arguments
/// * `scale` - Font scaling factor (1 = 8x8, 2 = 16x16, etc.)
/// * `base_addr` - VGA framebuffer base address (e.g., 0xffff_0000_ecd2_0000)
pub fn init(scale: usize, base_addr: usize) {
    // Display logo for 1 second
    display_logo(base_addr);
    delay_1s();
    
    // Initialize console
    let console = VgaConsole::new(base_addr, scale);
    VGA.init_once(SpinNoIrq::new(console));
    // Clear screen on initialization
    VGA.lock().clear();
}

/// Returns true if VGA is initialized
pub fn is_inited() -> bool {
    VGA.is_inited()
}

/// Writes a slice of bytes to the VGA console
pub fn write_bytes(s: &[u8]) {
    if VGA.is_inited() {
        VGA.lock().write_bytes(s);
    }
}

/// Writes a string to the VGA console
pub fn draw_string(s: &str) {
    write_bytes(s.as_bytes());
}

/// Clears the VGA screen
pub fn clear() {
    if VGA.is_inited() {
        VGA.lock().clear();
    }
}

/// Sets the font scale for the VGA console
pub fn set_font_scale(scale: usize) {
    if VGA.is_inited() {
        VGA.lock().set_font_scale(scale);
    }
}

/// Sets the foreground color
pub fn set_fg_color(color: u32) {
    if VGA.is_inited() {
        VGA.lock().set_fg_color(color);
    }
}

/// Sets the background color
pub fn set_bg_color(color: u32) {
    if VGA.is_inited() {
        VGA.lock().set_bg_color(color);
    }
}

/// Redraws the screen from the log buffer
pub fn redraw_from_log() {
    if VGA.is_inited() {
        VGA.lock().redraw_from_log();
    }
}

/// Returns the number of cached log bytes
pub fn log_buffer_len() -> usize {
    if VGA.is_inited() {
        VGA.lock().log_buffer_len()
    } else {
        0
    }
}

/// Formats and writes arguments to the VGA console
pub fn draw_args(s: &core::fmt::Arguments) {
    use core::fmt::Write;
    struct VGAWriter;

    impl core::fmt::Write for VGAWriter {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            write_bytes(s.as_bytes());
            Ok(())
        }
    }

    let mut writer = VGAWriter;
    let _ = writer.write_fmt(*s);
}

/// Print macro for VGA output
#[macro_export]
macro_rules! vga_print {
    ($($arg:tt)*) => ({
        $crate::vga::draw_args(&format_args!($($arg)*));
    });
}

/// Println macro for VGA output
#[macro_export]
macro_rules! vga_println {
    () => ($crate::vga_print!("\n"));
    ($fmt:expr) => ($crate::vga_print!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => ($crate::vga_print!(concat!($fmt, "\n"), $($arg)*));
}
