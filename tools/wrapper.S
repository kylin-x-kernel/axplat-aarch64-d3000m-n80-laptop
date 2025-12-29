/*
 * wrapper.S - ARM64 EFI Stub Wrapper for ArceOS
 * 
 * 参考 Linux 内核 arch/arm64/kernel/head.S 和 efi-header.S
 * 
 * 作用：让 UEFI 模式下的 GRUB 能够加载 ArceOS
 * 原理：
 *   1. 替换 ArceOS 的 Linux Image Header，添加 PE/COFF 支持
 *   2. 保持 ArceOS 代码在相同的相对偏移
 * 
 * 重要：这个文件需要和 merge_wrapper.sh 脚本配合使用
 *       脚本会把 ArceOS bin 的代码部分（跳过前 64 字节 header）
 *       合并到 wrapper 的正确位置
 * 
 * 使用方法：
 *   1. aarch64-linux-gnu-gcc -c wrapper.S -o wrapper.o
 *   2. aarch64-linux-gnu-objcopy -O binary wrapper.o wrapper.bin
 *   3. 使用特殊的合并脚本（见下方说明）
 */

/* ============================================================
 * 常量定义
 * ============================================================ */

/* PE/COFF 常量 */
.set PE_MAGIC,                      0x00004550      /* "PE\0\0" */
.set IMAGE_FILE_MACHINE_ARM64,      0xAA64
.set PE_OPT_MAGIC_PE32PLUS,         0x020B

/* Characteristics */
.set IMAGE_FILE_EXECUTABLE_IMAGE,   0x0002
.set IMAGE_FILE_LINE_NUMS_STRIPPED, 0x0004
.set IMAGE_FILE_DEBUG_STRIPPED,     0x0200
.set IMAGE_FILE_CHARACTERISTICS,    IMAGE_FILE_EXECUTABLE_IMAGE | \
                                    IMAGE_FILE_LINE_NUMS_STRIPPED | \
                                    IMAGE_FILE_DEBUG_STRIPPED

/* Subsystem */
.set IMAGE_SUBSYSTEM_EFI_APPLICATION, 0x000A

/* Section Characteristics */
.set IMAGE_SCN_CNT_CODE,            0x00000020
.set IMAGE_SCN_MEM_EXECUTE,         0x20000000
.set IMAGE_SCN_MEM_READ,            0x40000000

/* 对齐和大小 */
.set SEGMENT_ALIGN,                 0x1000          /* 4KB 段对齐 */
.set FILE_ALIGN,                    0x200           /* 512B 文件对齐 */

/* ArceOS 相关偏移 (从 helloworld ELF 分析得出) */
.set ARCEOS_CODE_OFFSET,            0x2000          /* _start_primary 相对于 _start 的偏移 */

/* ============================================================
 * ARM64 Linux Image Header (offset 0x00 - 0x3F, 64 bytes)
 * 参考: Linux Documentation/arch/arm64/booting.rst
 * 
 * 这个 header 替换 ArceOS 原有的 header，但保持相同的布局
 * ============================================================ */

    .section ".head", "ax"
    .global _start

_start:
.L_head:
    /*
     * 0x00: code0
     * 
     * Linux 内核使用 "add x13, x18, #0x16" 指令，其机器码是 "MZ"
     */
    add     x13, x18, #0x16         /* 机器码 = 0x5A4D = "MZ" (小端) */

    /* 0x04: code1 - 跳转到 ArceOS 的 _start_primary
     * 
     * 这个跳转需要和 ArceOS 原始跳转保持相同的偏移！
     * ArceOS 原始: 从 0x04 跳到 0x1048，偏移 = 0x1044 / 4 = 0x411
     */
    b       .L_arceos_entry         /* 跳转到 0x2000 */

    /* 0x08: text_offset - Image 加载偏移 */
    .quad   0

    /* 0x10: image_size - 有效镜像大小 (会被合并脚本更新) */
    .quad   0x100000                /* 1MB 估计值 */

    /* 0x18: flags
     *   Bit 0:    Kernel endianness (0 = LE)
     *   Bit 1-2:  Page size (1 = 4K)
     *   Bit 3:    Physical placement (1 = 2MB aligned anywhere)
     */
    .quad   0xA                     /* LE, 4K pages, anywhere */

    /* 0x20 - 0x37: 保留字段 */
    .quad   0                       /* reserved */
    .quad   0                       /* reserved */
    .quad   0                       /* reserved */

    /* 0x38: magic - ARM64 魔数 */
    .ascii  "ARM\x64"

    /* 0x3C: pe_header_offset - PE 头的偏移 */
    .long   .L_pe_header - .L_head

/* ============================================================
 * PE/COFF Header (offset 0x40)
 * ============================================================ */

    .balign 4
.L_pe_header:
    /* PE Signature (4 bytes) */
    .long   PE_MAGIC

    /* COFF File Header (20 bytes) */
    .short  IMAGE_FILE_MACHINE_ARM64            /* Machine */
    .short  .L_section_count                    /* NumberOfSections */
    .long   0                                   /* TimeDateStamp */
    .long   0                                   /* PointerToSymbolTable */
    .long   0                                   /* NumberOfSymbols */
    .short  .L_section_table - .L_optional_header /* SizeOfOptionalHeader */
    .short  IMAGE_FILE_CHARACTERISTICS          /* Characteristics */

/* ============================================================
 * Optional Header (PE32+)
 * ============================================================ */

.L_optional_header:
    /* Standard COFF Fields */
    .short  PE_OPT_MAGIC_PE32PLUS               /* Magic: PE32+ */
    .byte   0x02                                /* MajorLinkerVersion */
    .byte   0x14                                /* MinorLinkerVersion */
    .long   0x200000                            /* SizeOfCode (2MB) */
    .long   0                                   /* SizeOfInitializedData */
    .long   0                                   /* SizeOfUninitializedData */
    .long   ARCEOS_CODE_OFFSET                  /* AddressOfEntryPoint = 0x2000 */
    .long   ARCEOS_CODE_OFFSET                  /* BaseOfCode */

    /* Windows-Specific Fields (PE32+) */
    .quad   0                                   /* ImageBase (relocatable) */
    .long   SEGMENT_ALIGN                       /* SectionAlignment */
    .long   FILE_ALIGN                          /* FileAlignment */
    .short  0                                   /* MajorOperatingSystemVersion */
    .short  0                                   /* MinorOperatingSystemVersion */
    .short  1                                   /* MajorImageVersion */
    .short  0                                   /* MinorImageVersion */
    .short  0                                   /* MajorSubsystemVersion */
    .short  0                                   /* MinorSubsystemVersion */
    .long   0                                   /* Win32VersionValue */
    .long   0x300000                            /* SizeOfImage (3MB) */
    .long   ARCEOS_CODE_OFFSET                  /* SizeOfHeaders */
    .long   0                                   /* CheckSum */
    .short  IMAGE_SUBSYSTEM_EFI_APPLICATION     /* Subsystem: EFI Application */
    .short  0                                   /* DllCharacteristics */
    .quad   0                                   /* SizeOfStackReserve */
    .quad   0                                   /* SizeOfStackCommit */
    .quad   0                                   /* SizeOfHeapReserve */
    .quad   0                                   /* SizeOfHeapCommit */
    .long   0                                   /* LoaderFlags */
    .long   (.L_section_table - .) / 8          /* NumberOfRvaAndSizes */

    /* Data Directories (每个 8 字节: VA + Size) */
    .quad   0                                   /* ExportTable */
    .quad   0                                   /* ImportTable */
    .quad   0                                   /* ResourceTable */
    .quad   0                                   /* ExceptionTable */
    .quad   0                                   /* CertificationTable */
    .quad   0                                   /* BaseRelocationTable */

/* ============================================================
 * Section Table
 * ============================================================ */

.L_section_table:
    /* .text Section Header (40 bytes) */
    .ascii  ".text\0\0\0"                       /* Name (8 bytes) */
    .long   0x200000                            /* VirtualSize (2MB) */
    .long   ARCEOS_CODE_OFFSET                  /* VirtualAddress */
    .long   0x200000                            /* SizeOfRawData (2MB) */
    .long   ARCEOS_CODE_OFFSET                  /* PointerToRawData */
    .long   0                                   /* PointerToRelocations */
    .long   0                                   /* PointerToLineNumbers */
    .short  0                                   /* NumberOfRelocations */
    .short  0                                   /* NumberOfLineNumbers */
    .long   IMAGE_SCN_CNT_CODE | \
            IMAGE_SCN_MEM_READ | \
            IMAGE_SCN_MEM_EXECUTE               /* Characteristics */

    /* 计算 section 数量 */
    .set    .L_section_count, (. - .L_section_table) / 40

/* ============================================================
 * 填充到 0x2000 (ArceOS _start_primary 入口)
 * ============================================================ */

    .balign 4
    .org    ARCEOS_CODE_OFFSET

.L_arceos_entry:
    /*
     * ArceOS 的 _start_primary 代码会从这里开始
     * 
     * 合并脚本需要：
     *   1. 跳过 ArceOS bin 的前 0x1048 字节 (header + padding)
     *   2. 把剩余代码追加到这里
     * 
     * 命令：
     *   dd if=arceos.bin bs=1 skip=4168 >> wrapper.bin
     *   (4168 = 0x1048)
     */

.L_end:
