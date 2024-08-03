<center>
# **Endless Hole: Interrupt Handler Virtual Machine** #
</center>

This document describes the virtual machine that executes interrupt handler programs on the behalf of drivers in the system.

The purpose of the VM is allow device drivers to provide code to run directly in interrupt handlers to improve latency without compromising on process isolation.
Programs in the VM should be easily verified that they don't misbehave, i.e. read/write to arbitrary memory or never halt.
This makes the system more reliable and extensible, and keeps the kernel small.
By running the programs directly in the kernel, we avoid the performance overhead of a context switch into the driver process.

# Model
This VM has limited access to its parent driver's address space, and could run concurrently with its parent like a thread.
The VM is register based with 16 64-bit registers named `A0`-`A15`.
The VM can also access 8 pre-specified memory regions named `S` and `R1`-`R7`.
The `S` region is scratch space that is reset on each VM creation, but may not be zeroed.
Driver processes optionally provide slices into their own address space for each of `R1`-`R7`, which much stay alive for the duration of the lifetime of the handler.

A new VM instance is created each time an interrupt is handled.
When the VM starts, information about the interrupt it is handling is loaded into the registers.
Execution begins at the start of the program instruction blob and continues until a halt, panic or the end of the blob is reached.
TODO: which ones and what data?

The VM allows the program to copy data between regions, read and write from regions with the option for atomicity, and send messages to processes.

# Instructions

## General Encoding
Each instruction is encoded as a little-endian 4 byte value, with a 7 bit opcode.

Registers are encoded using 4 bits to index into the 16 different registers.

Regions are encoded using 3 bits, with the `S` region being `0b000`, and each of `R1`-`R7` mapped to `0b001`-`0b111`.

All bit ranges given are **inclusive**.
Any bits that are not given a value are reserved and should be cleared to 0.

### Opcodes
| Instruction       | Opcode Value |
|-------------------|--------------|
| `nop`             | `0b000_0000` |
| `move`            | `0b000_0001` |
| `load`            | `0b000_0010` |
| `load imm`        | `0b000_0011` |
| `store`           | `0b000_0100` |
| arith./compare    | `0b000_0101` |
| `branch`          | `0b000_0110` |
| `loop`            | `0b000_0111` |
| `send`            | `0b000_0000` |
| `copy`            | `0b000_0000` |
| `length_of`       | `0b000_0000` |
| `halt`            | `0b000_0000` |
| `debug_log`       | `0b000_0000` |
| `panic`           | `0b000_0000` |

## `move`

The `move` instruction moves data between registers.

### Parameters
- Source register
- Destination register

### Encoding

| Bit Range | Parameter         |
|-----------|-------------------|
| `7 : 0`   | Opcode            |
| `27 : 24` | Destination register |
| `31 : 28` | Source register |

## `load` / `store`

The `load` instruction reads a data from a memory region with an offset into a register.
If indexed mode is enabled, then the address read is `base + (index << stride)`.

The `store` instruction has the same shape, but writes data from a register into a memory region.
This means that in the following tables, for `store`, the "Source" and "Destination" are swapped.

### Parameters
- Destination register
- Source region
- Source base offset register
- (indexed mode only) Source index register
- (indexed mode only) Stride value

### Encoding

| Bit Range | Parameter         |
|-----------|-------------------|
| `7 : 0`   | Opcode            |
| `8`       | Enable indexed mode |
| `15 : 9`  | Stride value      |
| `18 : 16` | Source region     |
| `23 : 19` | Source index register |
| `27 : 24` | Source base offset register |
| `31 : 28` | Destination register |

## `load immediate`

The `load immediate` instruction loads data from the instruction stream into a register.
Values are always writen into the lowest bits of a register.

### Parameters
- Destination register
- Value
- Value size/Variant
    + 16 bit, 32 bit, 48 bit or 64 bit value
    + Zero remaining bits or leave them

### Encoding

Unlike all other instructions, this instruction may read up to 8 more bytes past the end of its 4 byte encoding.
These bytes make up the value for 32, 48 and 64 bit variants of this instruction.
For the 48 bit variant, both the two bytes in the instruction and 4 extra bytes after are used for the value.

| Bit Range | Parameter |
|-----------|-----------|
| `7 : 0`   | Opcode    |
| `11 : 8`  | Variant (See Variant Encoding table) |
| `15 : 12` | Destination register |
| `31 : 15` | Value for 16bit variant, highest two bytes in 48bit variant |

### Variants
| Value | Size         | Zero/Retain Remaining |
|-------|--------------|-----------------------|
| 0000b | 16 bit value | Zero |
| 0001b | 32 bit value | Zero |
| 0010b | 48 bit value | Zero |
| 0011b | 64 bit value | Zero |
| 0100b | 16 bit value | Retain |
| 0101b | 32 bit value | Retain |
| 0110b | 48 bit value | Retain |
| 0111b | 64 bit value | Retain |

## Arithmetic/Comparison
The arithmetic and comparison operations all have the same encodings, varying only in the operation they perform.
Divide by zero results in a panic with error code `0x0001_0000_0000_0000` 

### Parameters
- Instruction Variant Code
- First argument (A) register
- Second argument (B) register
- Output register (X)

### Variants
| Operation              | Description                                                                                          | Variant Code   |
|------------------------|------------------------------------------------------------------------------------------------------|----------------|
| `add`                  | Adds A and B, wrapping on overflow                                                                   | `0b0000_0000`  |
| `sub`                  | Subtracts A from B, wrapping on overflow                                                             | `0b0000_0001`  |
| `mul`                  | Multiplies A and B, wrapping on overflow                                                             | `0b0000_0010`  |
| `div`                  | Divides A by B, rounding down.                                                                       | `0b0000_0011`  |
| `mod`                  | Computes the remainder of dividing A by B, rounding down.                                            | `0b0000_0100`  |
| `and`                  | Bitwise And operation of A and B                                                                     | `0b0000_0101`  |
| `or`                   | Bitwise Or operation of A and B                                                                      | `0b0000_0110`  |
| `xor`                  | Bitwise Exclusive Or operation of A and B                                                            | `0b0000_0111`  |
| `shift left`           | Shifts bits left of A by B bits, zero extended                                                       | `0b0000_1000`  |
| `shift right`          | Shifts bits right of A by B bits, zero extended                                                      | `0b0000_1001`  |
| `arithmetic shift right` | Shifts bits right of A by B bits, sign bit extended                                                | `0b0000_1010`  |
| `invert`               | Flips the bits in the argument A, ignores B                                                          | `0b0000_1011`  |

### Encoding

| Bit Range | Parameter         |
|-----------|-------------------|
| `7 : 0`   | Opcode            |
| `18 : 10` | Variant Code |
| `23 : 19` | X register |
| `27 : 24` | B register |
| `31 : 28` | A register |

## `branch`
Chooses between two code paths given a test on a register value.
The instruction continues to the next instruction if the test fails, otherwise it jumps to the instruction indicated by adding the offset to the instruction pointer.
It is not possible to use `branch` to jump backwards in the instruction stream.

### Parameters
- Test Kind
- Test Register (T)
- Destination offset instruction count (Note: `load imm` instructions can count for two instructions due to their encoding)

### Encoding

| Bit Range | Parameter         |
|-----------|-------------------|
| `7 : 0`   | Opcode            |
| `9  : 7`  | Test Kind         |
| `14 : 10` | Test Register     |
| `31 : 15` | Destination offset|

### Test Kinds

| Description        | Encoding |
|--------------------|----------|
| Always             | `0b000`  |
| T = 0              | `0b001`  |
| T ≠ 0              | `0b010`  |
| T < 0              | `0b011`  |
| T > 0              | `0b100`  |
| T ≤ 0              | `0b101`  |
| T ≥ 0              | `0b110`  |

# `loop`
Repeats a section of code for a certain number of repetitions.
The repeat count is fixed at the start of the loop.

## `send`

The `send` instruction sends a message to a process. The message must be previously constructed in one of the available memory regions.

### Parameters
- Memory region containing the message
- Offset to beginning of the message
- Length of the message in message blocks
- Register that will receive the ID of the sent message or zero if there was an error sending the message

### Encoding

| Bit Range | Parameter         |
|-----------|-------------------|
| `7 : 0`   | Opcode            |
| `21 : 18` | output message ID/error register |
| `23 : 20` | Memory region containing message |
| `27 : 24` | Offset register |
| `31 : 28` | Length register |

## `copy`

The `copy` instruction copies some number of bytes between regions, with a source and destination offset.

### Parameters
- Source region where copy will read from
- Destination region where copy will write to
- Source offset register relative to the beginning of the region
- Destination offset register relative to the beginning of the region
- Register containing the number of bytes to copy starting from the offset, or the length of the copy

### Encoding

| Bit Range | Parameter         |
|-----------|-------------------|
| `7 : 0`   | Opcode            |
| `15 : 13` | Source region     |
| `18 : 16` | Destination region |
| `23 : 19` | Source offset register |
| `27 : 24` | Dest. offset register |
| `31 : 28` | Length register |

## `length_of`

The `length_of` instruction reads the length in bytes of a memory region into a register.

### Parameters
- Region to consider
- Register to place length in

### Encoding

| Bit Range | Parameter         |
|-----------|-------------------|
| `7 : 0`   | Opcode            |
| `27 : 25` | Region to measure |
| `31 : 28` | Length Output Register |

## `halt`

The `halt` instruction causes the handler to halt, ending the processing of the current interrupt by the driver.

### Parameters
This instruction has no parameters.

### Encoding

| Bit Range | Parameter         |
|-----------|-------------------|
| `7 : 0`   | Opcode            |

## `debug_log`

The `debug_log` instruction causes the kernel to print out debugging information to the kernel log about the state of the VM and its parent driver process.

This information includes:

- Current register values
- Current instruction pointer
- Relevant scratch values
- PID of the driver
- Which interrupt caused this handler to be invoked

This instruction is ignored in non-debug mode kernels.

### Parameters

- A 24-bit tag code printed verbatim to the kernel log to identify the place in the handler responsible for the log output

### Encoding

| Bit Range | Parameter         |
|-----------|-------------------|
| `7 : 0`   | Opcode            |
| `31 : 9`  | Tag               |

## `panic`

The `panic` instruction causes the handler to halt and the kernel to print out information similar to the `debug_log` instruction, in addition to the panic code provided by the handler.
A message will also be sent to the responsible driver to inform it that the handler panicked.
Information printed may be reduced in non-debug mode kernels.

### Parameters

- A 24-bit code printed verbatim to the kernel log

### Encoding

| Bit Range | Parameter         |
|-----------|-------------------|
| `7 : 0`   | Opcode            |
| `31 : 9`  | Error code        |

