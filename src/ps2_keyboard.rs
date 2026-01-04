use crate::driver_input::{InputDriverOps, InputEvent};
use lazyinit::LazyInit;
use log::info;

const DATA_PORT_OFFSET: usize = 0x60;
const STATUS_PORT_OFFSET: usize = 0x64;
const STATUS_OUTPUT_FULL: u8 = 0x01;

pub static KBD: LazyInit<Ps2Keyboard> = LazyInit::new();

pub struct Ps2Keyboard {
    base_vaddr: usize,
}

impl Ps2Keyboard {
    pub fn new(base_vaddr: usize) -> Self {
        Self {
            base_vaddr,
        }
    }

    fn read_status(&self) -> u8 {
        unsafe { ((self.base_vaddr + STATUS_PORT_OFFSET) as *const u32).read_volatile() as u8 }
    }

    fn read_data(&self) -> u8 {
        unsafe { ((self.base_vaddr + DATA_PORT_OFFSET) as *const u32).read_volatile() as u8 }
    }

    fn write_data(&self, data: u8) {
        // Wait for Input Buffer Empty (bit 1 == 0)
        let mut timeout = 100000;
        while (self.read_status() & 0x02) != 0 && timeout > 0 {
            timeout -= 1;
            core::hint::spin_loop();
        }
        if timeout == 0 {
            info!("PS/2 KBD: Write timeout (IBF stuck)!");
            return;
        }
        unsafe { ((self.base_vaddr + DATA_PORT_OFFSET) as *mut u32).write_volatile(data as u32) }
    }

    pub fn init_hw(&self) {
        info!("PS/2 KBD: Initializing at vaddr {:#x}...", self.base_vaddr);
        
        let status = self.read_status();
        info!("PS/2 KBD: Initial status: {:#x}", status);

        if status == 0xFF {
            info!("PS/2 KBD: Status is 0xFF, device might not be present or mapped correctly.");
            return;
        }

        // 1. Flush output buffer
        let mut flush_count = 0;
        while (self.read_status() & STATUS_OUTPUT_FULL) != 0 && flush_count < 1000 {
            let _ = self.read_data();
            flush_count += 1;
        }
        if flush_count >= 1000 {
            info!("PS/2 KBD: Flush timeout! Status: {:#x}", self.read_status());
        }

        // 2. Send Enable Scanning (0xF4)
        self.write_data(0xF4);
        
        // 3. Wait for ACK (0xFA)
        let mut timeout = 100000;
        while timeout > 0 {
            if (self.read_status() & STATUS_OUTPUT_FULL) != 0 {
                let data = self.read_data();
                if data == 0xFA {
                    info!("PS/2 KBD: Enabled successfully (ACK received).");
                    break;
                } else {
                    info!("PS/2 KBD: Unexpected response: {:#x}", data);
                }
            }
            timeout -= 1;
            core::hint::spin_loop();
        }
        if timeout == 0 {
            info!("PS/2 KBD: Enable timeout!");
        }
    }

}

impl InputDriverOps for Ps2Keyboard {
    fn pending_input(&self) -> bool {
        (self.read_status() & STATUS_OUTPUT_FULL) != 0
    }

    fn read_event(&self) -> Option<InputEvent> {
        if self.pending_input() {
            let scancode = self.read_data();
            if let Some(ascii) = ps2_scancode_to_ascii(scancode) {
                return Some(InputEvent::KeyPress(ascii));
            }
        }
        None
    }
}

pub fn init(base_vaddr: usize) {
    KBD.init_once(Ps2Keyboard::new(base_vaddr));
    KBD.init_hw();
}

fn ps2_scancode_to_ascii(scancode: u8) -> Option<u8> {
    match scancode {
        0x1E => Some(b'a'), 0x30 => Some(b'b'), 0x2E => Some(b'c'), 0x20 => Some(b'd'),
        0x12 => Some(b'e'), 0x21 => Some(b'f'), 0x22 => Some(b'g'), 0x23 => Some(b'h'),
        0x17 => Some(b'i'), 0x24 => Some(b'j'), 0x25 => Some(b'k'), 0x26 => Some(b'l'),
        0x32 => Some(b'm'), 0x31 => Some(b'n'), 0x18 => Some(b'o'), 0x19 => Some(b'p'),
        0x10 => Some(b'q'), 0x13 => Some(b'r'), 0x1F => Some(b's'), 0x14 => Some(b't'),
        0x16 => Some(b'u'), 0x2F => Some(b'v'), 0x11 => Some(b'w'), 0x2D => Some(b'x'),
        0x15 => Some(b'y'), 0x2C => Some(b'z'),
        0x02 => Some(b'1'), 0x03 => Some(b'2'), 0x04 => Some(b'3'), 0x05 => Some(b'4'),
        0x06 => Some(b'5'), 0x07 => Some(b'6'), 0x08 => Some(b'7'), 0x09 => Some(b'8'),
        0x0A => Some(b'9'), 0x0B => Some(b'0'),
        0x1C => Some(b'\r'), // Enter
        0x39 => Some(b' '),  // Space
        0x0E => Some(0x08),  // Backspace
        _ => None,
    }
}
