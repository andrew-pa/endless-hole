# Kernel
This document describes the Endless Hole microkernel.

The kernel is responsible for managing the most fundamental system mechanisms: time and space, or in other words: thread scheduling and memory mapping/allocation.
In addition, the kernel provides a mechanism for processes in the system to communicate with each other through messages and shared memory.
Finally, the kernel also takes care of handling interrupts from devices and notifying the responsible driver.

## Overview
### Processes and Threads
#### Processes
A process is a collection of threads who share the same:
- process ID
- memory address space and memory loans
- supervisor process ID
- role (process level and supervisor status)

Processes start with a single main thread running at their entry point.
Processes run until they exit or encounter a fault.
When a process exits for any reason, the parent of the process can be notified.

#### Process Roles
The supervisor process is the process responsible for resource resolution in the supervised process.
Processes resolve resources (i.e. discover the PIDs of services in the system) by sending their supervisor a message.
The supervisor process is inherited by spawned child processes.

Each process has a privilege level of "driver", "privileged", or "unprivileged".
Unprivileged processes cannot send messages to other processes outside their supervisor's children.
Privileged processes can send messages to other processes outside their supervisor's children, but not outside their supervisor's supervisor's children.
A driver process can send messages to any process in the system.
Processes inherit the privilege level of their parent, unless their parent gives them a lower privilege level when spawned.

Supervisor processes and the privilege level system enable the creation of new resource scopes, where access to the rest of the system is totally mediated via the supervisor.
This is similar to containers, although technically much more flexible.

#### Threads
A thread is a single path of execution in a process, and has its own:
- program counter/CPU state
- stack
- message queue

Threads are scheduled by the kernel for execution on the available CPUs in the system.
Thread IDs are local to their process, a (process ID, thread ID) pair uniquely identifies a thread.
A single thread in each process is designated as the receiver thread for the process, and will receive messages from other processes who send messages to its process without a thread ID. By default, this is the main thread.

### Memory
Each process has its own virtual address space managed by the kernel.
When a process is created, the address space contains the loaded executable binary, the stack, and any initial parameters.
All processes can request new pages of RAM from the kernel to be mapped into their address space for heap purposes.

The Rust borrowing idea is extended to interprocess shared memory - a memory "loan".
Loans can either be shared and read-only (like a `&T`) or exclusive and read-write (like a `&mut T`).
A loan consists of the base address in the loaning process and the number of pages to be loaned.
The loan's extent must be an even number of pages.
Driver processes can also request a loan from the kernel for an arbitrary region of physical addresses.

In addition to loans, processes can move memory from their own process to another process.

The kernel's own virtual memory is identity mapped to cover the whole range of physical memory.

### Messages
The kernel distributes messages between threads.
Messages consist of 64-byte blocks, and can be a maximum of 16 blocks long (1024 bytes).
Messages contain:
- the process and thread ID of the sender
- flags indicating if this message is a reply and if it contains a memory operation
- the number of blocks total used by the message
- a unique message ID
- optionally, the unique message ID this message is in response to
- optionally, a memory loan or move that can be accepted by the receiver
- some amount of data

Messages are queued but if the queue overflows, the process will fault.
Messages can be sent to a process' designated receiver thread without knowing the thread ID.

### Boot Process
The kernel boot process looks something like:
- Parse the device tree blob and kernel arguments from U-boot to determine the hardware configuration
- Initialize core devices
    - CPU
    - Debug logging via UART
    - Memory
        - page tables
        - allocator
    - Interrupt controller and interrupt handlers
    - Timers
    - Thread scheduler
- Locate and parse the initramfs blob
- Load the `init` process from the initramfs and spawn it. The `init` process is loaned the device tree blob and initramfs blob, and starts with 'driver' permissions.
- Start the thread scheduler


## Interfaces
The kernel's interface is primarily the system call interface. Additionally, the kernel processes some configuration provided by the firmware via the device tree blob.

### Devicetree Blob
See the [Devicetree Specification](https://github.com/devicetree-org/devicetree-specification) for more details.
The kernel aims to interpret values in the blob as defined by the standard wherever possible to discover the devices present in the system.

#### `/chosen/`
See Section 3.6 of the specification for more details.

This node may contain a kernel "command line" value in the `bootargs` property.
This value will be parsed as JSON and may contain the following keys:

- (TODO)

This node may also contain a `stdout-path` property. If present, this device will be the first choice for output from the kernel's debug UART logger.

### System Calls
The primary user space interface for the kernel is system calls.
System calls are made using the normal Aarch64 system call calling convention.
All system calls return zero on success, and an error code on failure. Any other outputs are returned via pointers.

TODO: should we use the immediate system call instruction value or pass the system call number via a register (like Linux?).


## Implementation Thoughts
This section is just some thoughts about implementation details. Things may or may not turn out like this.

### Messages
Messages should be short enough to be copied in a few instructions.
A single `LD1` instruction can load up to 64 bytes (using four actual loads), so messages should probably be about 64 bytes.
If process ID (4 bytes), thread ID (2 bytes), flags (2 bits needed), and message block count (4 bits needed) take up 8 bytes, then that leaves 56 bytes.
If the message ID takes up 4 bytes, then the message IDs can take at most 8 bytes, leaving at least 48 bytes.
A loan/move takes up 16 bytes, 8 bytes for the address and 8 bytes for the length, leaving at least 32 bytes for additional information.
All the remaining blocks in the message can be exclusively used for message payload.

The kernel must store messages that are in transit, having been sent but not yet received.
We could do this in a unified "message heap" that contains all messages, and then store for each thread a queue of slices into this heap.

### Device Driver Servers
A typical device driver server would be spawned as a 'driver' type process by the `init` process.
The driver would first request loans from the kernel or its lower-level driver, set up interrupts with the kernel, and then initialize the device.
The driver then listens for requests from clients, and handles them using the device. Ideally most actual data transfers happen using loans.
