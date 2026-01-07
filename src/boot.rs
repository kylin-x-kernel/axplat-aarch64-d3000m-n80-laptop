use axplat::mem::{Aligned4K, pa};
use page_table_entry::{GenericPTE, MappingFlags, aarch64::A64PTE};

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
unsafe extern "C" fn _start() {
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

/// Relocate the kernel to the specific address.
#[unsafe(naked)]
unsafe extern "C" fn relocate_self(target_addr: usize, dtb: usize) {
    core::arch::naked_asm!("
        // Save LR and args to callee-saved registers
        mov     x19, x30
        mov     x20, x0                 // x20 = target
        mov     x21, x1                 // x21 = dtb

        // Calculate current physical address
        adrp    x22, _skernel
        add     x22, x22, :lo12:_skernel  // x22 = current_base

        // Check if relocation is needed
        cmp     x20, x22
        b.eq    2f                      // If target == current, return

        // Calculate copy size
        adrp    x3, _ekernel
        add     x3, x3, :lo12:_ekernel
        sub     x3, x3, x22             // x3 = size

        // Copy loop
        mov     x11, x20                // x11 = dst
        mov     x12, x22                // x12 = src
1:      ldr     x4, [x12], #8
        str     x4, [x11], #8
        subs    x3, x3, #8
        b.gt    1b

        // Flush caches
        ic      iallu
        dsb     sy
        isb

        // Jump to new location
        mov     x0, x21                 // Restore DTB
        br      x20                     // Branch to target_addr

2:      mov     x30, x19                // Restore LR
        ret
    ",
    )
}

/// The earliest entry point for the primary CPU.
#[unsafe(naked)]
unsafe extern "C" fn _start_primary() {
    // X0 = dtb
    core::arch::naked_asm!("
        mov     x20, x0                 // save DTB pointer (callee-saved)
        mov     x0, #0x8000
        lsl     x0, x0, #16             // x0 = 0x8000_0000 (target address)
        mov     x1, x20                 // x1 = dtb
        bl      {relocate_self}          // relocate_self(0x8000_0000, dtb)

        mrs     x19, mpidr_el1
        and     x19, x19, #0xffffff     // get current CPU id
        mov     x20, x0                 // save DTB pointer

        adrp    x8, {boot_stack}        // setup boot stack
        add     x8, x8, {boot_stack_size}
        mov     sp, x8

        bl      {switch_to_el1}         // switch to EL1
        bl      {enable_fp}             // enable fp/neon

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
        init_mmu = sym axcpu::init::init_mmu,
        init_boot_page_table = sym init_boot_page_table,
        enable_fp = sym enable_fp,
        boot_pt = sym BOOT_PT_L0,
        phys_virt_offset = const PHYS_VIRT_OFFSET,
        boot_stack = sym BOOT_STACK,
        boot_stack_size = const BOOT_STACK_SIZE,
        relocate_self = sym relocate_self,
        entry = sym axplat::call_main
    )
}

/// The earliest entry point for the secondary CPUs.
#[cfg(feature = "smp")]
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn _start_secondary() {
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
        init_mmu = sym axcpu::init::init_mmu,
        enable_fp = sym enable_fp,
        boot_pt = sym BOOT_PT_L0,
        phys_virt_offset = const PHYS_VIRT_OFFSET,
        entry = sym axplat::call_secondary_main,
    )
}