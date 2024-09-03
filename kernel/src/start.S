
    /* start.S: the very earliest boot phase */

.section .text.boot

.global _start

_start:
    /* disable MMU */
    mrs x1, SCTLR_EL1
    bic x1, x1, #1
    msr SCTLR_EL1, x1
    isb

    adr x1, _kernel_page_table_root

    /* create correct level 0 entry to point to our level 1 table */
    adr x2, _kernel_id_map_level1_table
    /* lsr x1, x1, #12 */
    bic x2, x2, #0xfff
    orr x2, x2, #3
    str x2, [x1]

    /* set TTLB0/1 to fixed(?) map that sets up the kernel mapping correctly and identity maps the next instructions*/
    msr TTBR0_EL1, x1
    msr TTBR1_EL1, x1

    /* set TCR to known correct value 0x0784'8410'3510*/
    mov x1, #0x3510
    movk x1, #0x8410, lsl 16
    movk x1, #0x0784, lsl 32
    msr TCR_EL1, x1

    /* enable the MMU */
    mrs x1, SCTLR_EL1
    orr x1, x1, #1
    msr SCTLR_EL1, x1
    isb

    /* setup stack pointer */
    /* the stack is >1MB away, so we need to load the page address first */
    adrp x1, __stack_start
    add x1, x1, :lo12:__stack_start
    movk x1, #0xffff, lsl 48 /* adjust for virtual addressing */
    mov sp, x1

    /* start the kernel, now running in the correct spot in virtual memory */
    adr x1, kmain
    movk x1, #0xffff, lsl 48
    br x1
    /* we cannot come back here, but just in case we do */
halt:
    b halt

.section .data.boot

.global _kernel_page_table_root

.balign 4096
_kernel_page_table_root:
    .quad 0
    .ds.d 511

.balign 4096
_kernel_id_map_level1_table:
    .quad (0x00000000) | (1 << 10) | 1
    .quad (0x40000000) | (1 << 10) | 1
    .quad (0x80000000) | (1 << 10) | 1
    .quad (0xc0000000) | (1 << 10) | 1
    .ds.d 508
