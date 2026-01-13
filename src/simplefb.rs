use kspin::SpinNoIrq;
use lazyinit::LazyInit;
use simplefb::{FramebufferConfig, LogBuffer, SimpleFbConsole};

extern crate alloc;

static SIMPLEFB: LazyInit<SpinNoIrq<SimpleFbConsole>> = LazyInit::new();
static mut LOG_BUFFER_STORAGE: [u8; 64 * 1024] = [0; 64 * 1024];
const LOGO_PNG: &[u8] = include_bytes!("../assets/arceos.png");

/// Display the picture centered on a white background
fn display_logo(config: &FramebufferConfig, width: usize, height: usize, data: &[u32]) {
    // Fill screen with white background
    unsafe {
        let ptr = config.base_addr as *mut u32;
        let total_pixels = config.width * config.height;
        for i in 0..total_pixels {
            core::ptr::write_volatile(ptr.add(i), 0xFFFFFF); // White
        }
    }

    // Calculate centered position
    let x_offset = (config.width.saturating_sub(width)) / 2;
    let y_offset = (config.height.saturating_sub(height)) / 2;

    simplefb::picture::draw_picture(config, x_offset, y_offset, width, height, data);
}

/// Delay function (simple busy-wait)
fn simple_delay(count: usize) {
    // Delay 1s
    let freq: u64;
    let start: u64;
    unsafe {
        core::arch::asm!("mrs {}, cntfrq_el0", out(reg) freq);
        core::arch::asm!("mrs {}, cntpct_el0", out(reg) start);
    }
    let target = start.wrapping_add(freq.wrapping_mul(count as u64));
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

/// Decode embedded PNG data
/// Return: (width, height, pixel_data)
fn decode_png(bytes: &[u8]) -> Option<(usize, usize, alloc::vec::Vec<u32>)> {
    if let Ok(header) = minipng::decode_png_header(bytes) {
        let mut buffer = alloc::vec![0u8; header.required_bytes()];
        if let Ok(image) = minipng::decode_png(bytes, &mut buffer) {
            let width = image.width() as usize;
            let height = image.height() as usize;
            let pixels = image.pixels();

            // Convert to u32 (ARGB/BGRA)
            let mut data = alloc::vec![0u32; width * height];

            match image.color_type() {
                minipng::ColorType::Rgba => {
                    for i in 0..width * height {
                        if i * 4 + 3 < pixels.len() {
                            let r = pixels[i * 4] as u32;
                            let g = pixels[i * 4 + 1] as u32;
                            let b = pixels[i * 4 + 2] as u32;
                            let a = pixels[i * 4 + 3] as u32;

                            // Alpha blending with white background
                            let bg = 255u32;
                            let r = (r * a + bg * (255 - a)) / 255;
                            let g = (g * a + bg * (255 - a)) / 255;
                            let b = (b * a + bg * (255 - a)) / 255;

                            data[i] = (r << 16) | (g << 8) | b;
                        }
                    }
                }
                minipng::ColorType::Rgb => {
                    for i in 0..width * height {
                        if i * 3 + 2 < pixels.len() {
                            let r = pixels[i * 3] as u32;
                            let g = pixels[i * 3 + 1] as u32;
                            let b = pixels[i * 3 + 2] as u32;
                            data[i] = (r << 16) | (g << 8) | b;
                        }
                    }
                }
                minipng::ColorType::GrayAlpha => {
                    for i in 0..width * height {
                        if i * 2 + 1 < pixels.len() {
                            let gray = pixels[i * 2] as u32;
                            let a = pixels[i * 2 + 1] as u32;
                            let bg = 255u32;
                            let val = (gray * a + bg * (255 - a)) / 255;
                            data[i] = (val << 16) | (val << 8) | val;
                        }
                    }
                }
                _ => {}
            }
            return Some((width, height, data));
        }
    }
    None
}

fn show_logo(config: &FramebufferConfig) {
    if let Some((width, height, data)) = decode_png(LOGO_PNG) {
        display_logo(config, width, height, &data);
        simple_delay(1); // Display for 1 seconds
    }
}

pub fn init(config: FramebufferConfig) {
    // Decode and display logo
    show_logo(&config);

    // Initialize LogBuffer
    // SAFETY: We are in initialization code, single threaded context assumed or handled by caller
    let storage = core::ptr::addr_of!(LOG_BUFFER_STORAGE).cast_mut();
    // SAFETY: storage is valid and has static lifetime
    let log_buffer = LogBuffer::new(storage);

    let console = SimpleFbConsole::new(config, log_buffer);
    SIMPLEFB.init_once(SpinNoIrq::new(console));
    SIMPLEFB.lock().clear();
}

pub fn write_bytes(bytes: &[u8]) {
    if SIMPLEFB.is_inited() {
        SIMPLEFB.lock().write_bytes(bytes);
    }
}
