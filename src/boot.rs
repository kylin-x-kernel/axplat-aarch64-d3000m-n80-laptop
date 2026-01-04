use core::arch::asm;

use axplat::mem::{Aligned4K, pa};
use page_table_entry::{GenericPTE, MappingFlags, aarch64::A64PTE};

use aarch64_cpu::{asm::barrier, registers::*};
use memory_addr::{PhysAddr, VirtAddr};
use crate::config::plat::{BOOT_STACK_SIZE, PHYS_VIRT_OFFSET};

#[unsafe(link_section = ".bss.stack")]
static mut BOOT_STACK: [u8; BOOT_STACK_SIZE] = [0; BOOT_STACK_SIZE];

#[unsafe(link_section = ".data.boot_page_table")]
static mut BOOT_PT_L0: Aligned4K<[A64PTE; 512]> = Aligned4K::new([A64PTE::empty(); 512]);

#[unsafe(link_section = ".data.boot_page_table")]
static mut BOOT_PT_L1: Aligned4K<[A64PTE; 512]> = Aligned4K::new([A64PTE::empty(); 512]);

unsafe fn init_boot_page_table() {
    unsafe {
        // 0x0000_0000_0000 ~ 0x0080_0000_0000, table
        BOOT_PT_L0[0] = A64PTE::new_table(pa!(&raw mut BOOT_PT_L1 as usize));
        BOOT_PT_L1[0] = A64PTE::new_page(
            pa!(0),
            MappingFlags::READ | MappingFlags::WRITE | MappingFlags::DEVICE,
            true,
        );
        // 0x0000_4000_0000..0x0000_8000_0000, 1G block, normal memory
        BOOT_PT_L1[1] = A64PTE::new_page(
            pa!(0x4000_0000),
            MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE,
            true,
        );

        // 0x0000_8000_0000..0x0000_C000_0000, 1G block, normal memory_set
        BOOT_PT_L1[2] = A64PTE::new_page(
            pa!(0x8000_0000),
            MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE,
            true,
        );

        // 0x0000_C000_0000..0x0001_0000_0000, 1G block, normal memory_set
        BOOT_PT_L1[3] = A64PTE::new_page(
            pa!(0xC000_0000),
            MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE,
            true,
        );
    }
}

unsafe fn enable_fp() {
    // FP/SIMD needs to be enabled early, as the compiler may generate SIMD
    // instructions in the bootstrapping code to speed up the operations
    // like `memset` and `memcpy`.
    #[cfg(feature = "fp-simd")]
    axcpu::asm::enable_fp();
}

/// Kernel entry point with Linux image header.
///
/// Some bootloaders require this header to be present at the beginning of the
/// kernel image.
///
/// Documentation: <https://docs.kernel.org/arch/arm64/booting.html>
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.boot")]
unsafe extern "C" fn _start() -> ! {
    const FLAG_LE: usize = 0b0;
    const FLAG_PAGE_SIZE_4K: usize = 0b10;
    const FLAG_ANY_MEM: usize = 0b1000;
    // PC = bootloader load address
    // X0 = dtb
    core::arch::naked_asm!("
        add     x13, x18, #0x16     // 'MZ' magic
        b       {entry}             // Branch to kernel start, magic

        .quad   0                   // Image load offset from start of RAM, little-endian
        .quad   _ekernel - _start   // Effective size of kernel image, little-endian
        .quad   {flags}             // Kernel flags, little-endian
        .quad   0                   // reserved
        .quad   0                   // reserved
        .quad   0                   // reserved
        .ascii  \"ARM\\x64\"        // Magic number
        .long   0                   // reserved (used for PE COFF offset)",
        flags = const FLAG_LE | FLAG_PAGE_SIZE_4K | FLAG_ANY_MEM,
        entry = sym _start_primary,
    )
}
/// The earliest entry point for the primary CPU.
#[unsafe(naked)]
unsafe extern "C" fn _start_primary() -> ! {
    // X0 = dtb
    core::arch::naked_asm!("
        mrs     x19, mpidr_el1
        and     x19, x19, #0xffffff     // get current CPU id
        mov     x20, x0                 // save DTB pointer
        adrp    x8, {boot_stack}        // setup boot stack
        add     x8, x8, {boot_stack_size}
        mov     sp, x8

        bl      {switch_to_el1}         // switch to EL1
        bl      {enable_fp}             // enable fp/neon
        
        // 传递 BOOT_PT_L0 的物理地址给 init_boot_page_table
        // adrp x0, {boot_pt} 计算出的就是物理地址
        adrp    x0, {boot_pt}
        bl      {init_boot_page_table}
        
        adrp    x0, {boot_pt}
        bl      {init_mmu}              // setup MMU
        
        mov     x8, {phys_virt_offset}  // set SP to the high address
        add     sp, sp, x8

        mov     x0, x19                 // call_main(cpu_id, dtb)
        mov     x1, x20
        ldr     x8, ={entry}
        blr     x8
        b      .",
        switch_to_el1 = sym axcpu::init::switch_to_el1,
        init_mmu = sym init_mmu,
        init_boot_page_table = sym init_boot_page_table,
        enable_fp = sym enable_fp,
        boot_pt = sym BOOT_PT_L0,
        phys_virt_offset = const PHYS_VIRT_OFFSET,
        boot_stack = sym BOOT_STACK,
        boot_stack_size = const BOOT_STACK_SIZE,
        // entry = sym test_main,
        entry = sym axplat::call_main
    )
}

/// The earliest entry point for the secondary CPUs.
#[cfg(feature = "smp")]
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn _start_secondary() -> ! {
    // X0 = stack pointer
    core::arch::naked_asm!("
        mrs     x19, mpidr_el1
        and     x19, x19, #0xffffff     // get current CPU id

        mov     sp, x0
        bl      {switch_to_el1}
        bl      {enable_fp}
        adrp    x0, {boot_pt}
        bl      {init_mmu}

        mov     x8, {phys_virt_offset}  // set SP to the high address
        add     sp, sp, x8

        mov     x0, x19                 // call_secondary_main(cpu_id)
        ldr     x8, ={entry}
        blr     x8
        b      .",
        switch_to_el1 = sym axcpu::init::switch_to_el1,
        init_mmu = sym init_mmu,
        enable_fp = sym enable_fp,
        boot_pt = sym BOOT_PT_L0,
        phys_virt_offset = const PHYS_VIRT_OFFSET,
        entry = sym axplat::call_secondary_main,
    )
}

// UART0 基地址 (QEMU virt 机器)
const UART0_BASE: usize = 0x000018002000;
// const UART0_BASE: usize = 0x09000000;

/// 打印字节
#[allow(unused)]
pub fn boot_serial_send(data: u8) {
    unsafe {
        let base = UART0_BASE as *mut u32;
        // PL011 Flag Register (FR) is at offset 0x18
        // Bit 5 is TXFF (Transmit FIFO Full)
        let fr = base.add(0x18 / 4);
        
        // Wait while TX FIFO is full
        while (core::ptr::read_volatile(fr) & (1 << 5)) != 0 {}
        
        core::ptr::write_volatile(base, data as u32);
    }
}

/// 打印字符串
#[unsafe(no_mangle)]
pub fn boot_print_str(data: &str) {
    for byte in data.bytes() {
        boot_serial_send(byte);
    }
}

/// 打印整形
#[allow(dead_code)]
pub fn boot_print_usize(num: usize) {
    _boot_print_usize(num);
}

/// 打印阶段标签，方便区分输出
#[unsafe(no_mangle)]
pub extern "C" fn boot_print_stage(stage: usize) {
    match stage {
        1 => boot_print_str("\r\n[EL2->EL1] before switch_to_el1\r\n"),
        2 => boot_print_str("\r\n[EL1] after switch_to_el1\r\n"),
        3 => boot_print_str("\r\n[EL1] after init_mmu\r\n"),
        4 => boot_print_str("\r\n[EL2->EL1 secondary] before switch_to_el1\r\n"),
        _ => boot_print_str("\r\n[stage]\r\n"),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _boot_print_usize(num: usize) {
    let mut msg: [u8; 16] = [0; 16];
    let mut num = num;
    let mut cnt = 0;

    boot_print_str("0x");
    if num == 0 {
        boot_serial_send('0' as u8);
    } else {
        loop {
            if num == 0 {
                break;
            }
            msg[cnt] = match (num & 0xf) as u8 {
                n if n < 10 => n + '0' as u8,
                n => n - 10 + 'a' as u8,
            };
            cnt += 1;
            num >>= 4;
        }
        for i in 0..cnt {
            boot_serial_send(msg[cnt - i - 1]);
        }
    }
    boot_print_str("\r\n");
}

/// BOOT阶段打印寄存器，用于调试
#[macro_export]
macro_rules! boot_print_reg {
    ($reg_name:tt) => {
        boot_print_str($reg_name);
        boot_print_str(": ");
        let reg;
        unsafe { core::arch::asm!(concat!("mrs {}, ", $reg_name), out(reg) reg) };
        boot_print_usize(reg);
    };
}

/// 打印EL1寄存器
#[allow(dead_code)]
pub fn print_el1_reg(switch: bool) {
    if !switch {
        return;
    }
    boot_print_str("\r\n=== EL1 Registers ===\r\n");
    boot_print_reg!("SCTLR_EL1");
    boot_print_reg!("SPSR_EL1");
    boot_print_reg!("TCR_EL1");
    boot_print_reg!("VBAR_EL1");
    boot_print_reg!("MAIR_EL1");
    boot_print_reg!("MPIDR_EL1");
    boot_print_reg!("TTBR0_EL1");
    boot_print_reg!("TTBR1_EL1");
    boot_print_reg!("ID_AA64AFR0_EL1");
    boot_print_reg!("ID_AA64AFR1_EL1");
    boot_print_reg!("ID_AA64DFR0_EL1");
    boot_print_reg!("ID_AA64DFR1_EL1");
    boot_print_reg!("ID_AA64ISAR0_EL1");
    boot_print_reg!("ID_AA64ISAR1_EL1");
    boot_print_reg!("ID_AA64ISAR2_EL1");
    boot_print_reg!("ID_AA64MMFR0_EL1");
    boot_print_reg!("ID_AA64MMFR1_EL1");
    boot_print_reg!("ID_AA64MMFR2_EL1");
    boot_print_reg!("ID_AA64PFR0_EL1");
    boot_print_reg!("ID_AA64PFR1_EL1");
    boot_print_str("=== End EL1 Registers ===\r\n\r\n");
}

/// 打印EL2寄存器
#[allow(dead_code)]
pub fn print_el2_reg(switch: bool) {
    if !switch {
        return;
    }
    boot_print_str("\r\n=== EL2 Registers ===\r\n");
    boot_print_reg!("SCTLR_EL2");
    boot_print_reg!("HCR_EL2");
    boot_print_reg!("TCR_EL2");
    boot_print_reg!("VBAR_EL2");
    boot_print_reg!("MAIR_EL2");
    boot_print_reg!("TTBR0_EL2");
    boot_print_reg!("VTCR_EL2");
    boot_print_reg!("VTTBR_EL2");
    boot_print_reg!("ID_AA64MMFR0_EL1");
    boot_print_reg!("ID_AA64MMFR1_EL1");
    boot_print_reg!("ID_AA64PFR0_EL1");
    boot_print_reg!("ID_AA64PFR1_EL1");
    boot_print_str("=== End EL2 Registers ===\r\n\r\n");
}

/// 向 UART 写入一个字符
fn uart_putc(c: u8) {
    boot_serial_send(c);
}

/// 打印字符串到 UART
fn uart_puts(s: &str) {
    boot_print_str(s);
}

/// Configures and enables the MMU on the current CPU.
///
/// It first sets `MAIR_EL1`, `TCR_EL1`, `TTBR0_EL1`, `TTBR1_EL1` registers to
/// the conventional values, and then enables the MMU and caches by setting
/// `SCTLR_EL1`.
///
/// # Safety
///
/// This function is unsafe as it changes the address translation configuration.
pub unsafe fn init_mmu(root_paddr: PhysAddr) {
     // print _start symbol address 
     _boot_print_usize(_start as usize);
    _boot_print_usize(root_paddr.as_usize());
    _boot_print_usize(&raw mut BOOT_PT_L0 as usize);
    use page_table_entry::aarch64::MemAttr;

    MAIR_EL1.set(MemAttr::MAIR_VALUE);

    // Enable TTBR0 and TTBR1 walks, page size = 4K, vaddr size = 48 bits, paddr size = 48 bits.
    let tcr_flags0 = TCR_EL1::EPD0::EnableTTBR0Walks
        + TCR_EL1::TG0::KiB_4
        + TCR_EL1::SH0::Inner
        + TCR_EL1::ORGN0::WriteBack_ReadAlloc_WriteAlloc_Cacheable
        + TCR_EL1::IRGN0::WriteBack_ReadAlloc_WriteAlloc_Cacheable
        + TCR_EL1::T0SZ.val(16);
    let tcr_flags1 = TCR_EL1::EPD1::EnableTTBR1Walks
        + TCR_EL1::TG1::KiB_4
        + TCR_EL1::SH1::Inner
        + TCR_EL1::ORGN1::WriteBack_ReadAlloc_WriteAlloc_Cacheable
        + TCR_EL1::IRGN1::WriteBack_ReadAlloc_WriteAlloc_Cacheable
        + TCR_EL1::T1SZ.val(16);
    TCR_EL1.write(TCR_EL1::IPS::Bits_48 + tcr_flags0 + tcr_flags1);
    barrier::isb(barrier::SY);

    // Set both TTBR0 and TTBR1
    let root_paddr = root_paddr.as_usize() as u64;
    TTBR0_EL1.set(root_paddr);
    TTBR1_EL1.set(root_paddr);

    // Flush the entire TLB
    flush_tlb(None);

    // Enable the MMU and turn on I-cache and D-cache
    SCTLR_EL1.modify(SCTLR_EL1::M::Enable + SCTLR_EL1::C::Cacheable + SCTLR_EL1::I::Cacheable);
    // Disable SPAN
    SCTLR_EL1.set(SCTLR_EL1.get() | (1 << 23));
    barrier::isb(barrier::SY);
}

fn test_main() -> ! {
    uart_puts("Hello, RSTiny World!\n");

    loop {}
}
#[inline]
pub fn flush_tlb(vaddr: Option<VirtAddr>) {
    if let Some(vaddr) = vaddr {
        const VA_MASK: usize = (1 << 44) - 1; // VA[55:12] => bits[43:0]
        #[allow(unused_variables)]
        let operand = (vaddr.as_usize() >> 12) & VA_MASK;

        #[cfg(not(feature = "arm-el2"))]
        unsafe {
            // TLB Invalidate by VA, All ASID, EL1, Inner Shareable

            use core::arch::asm;
            asm!("dsb sy; isb; tlbi vmalle1; dsb sy; isb")
        }
        #[cfg(feature = "arm-el2")]
        unsafe {
            // TLB Invalidate by VA, EL2, Inner Shareable
            asm!("tlbi vae2is, {}; dsb sy; isb", in(reg) operand)
        }
    } else {
        // flush the entire TLB
        #[cfg(not(feature = "arm-el2"))]
        unsafe {
            // TLB Invalidate by VMID, All at stage 1, EL1

            use core::arch::asm;
            asm!("tlbi vmalle1; dsb sy; isb")
        }
        #[cfg(feature = "arm-el2")]
        unsafe {
            // TLB Invalidate All, EL2
            asm!("tlbi alle2; dsb sy; isb")
        }
    }
}
