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
- exit state: running, successfully exited with some code, exited with a fault

Processes start with a single main thread running at their entry point.
Processes run until they exit or encounter a fault.
When a process exits for any reason, the parent of the process can be notified.
Processes exit successfully when their last thread exits, and have the exit code provided by this last exit.

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
- state: running, waiting for message

Threads are scheduled by the kernel for execution on the available CPUs in the system.
Each thread has a unique ID.
A single thread in each process is designated as the receiver thread for the process, and will receive messages from other processes who send messages to its process without a thread ID. By default, this is the main thread.

### Memory
Each process has its own virtual address space managed by the kernel.
When a process is created, the address space contains the loaded executable binary, the stack, and any initial parameters.
All processes can request new pages of RAM from the kernel to be mapped into their address space for heap purposes.

The Rust borrowing idea is extended to interprocess shared memory - a memory "loan".
Loans can either be shared and read-only (like a `&T`) or exclusive and read-write (like a `&mut T`).
A loan consists of the base address in the loaning process and the number of pages to be loaned.
The loan's extent must be an whole number of pages.
Driver processes can also request a loan from the kernel for an arbitrary region of physical addresses.

In addition to loans, processes can move memory from their own process to another process.

The kernel's own virtual memory is identity mapped to cover the whole range of physical memory.

### Messages
The kernel distributes messages between threads.
Messages consist of 64-byte blocks, and can be a maximum of 16 blocks long (1024 bytes).
Messages must be 8-byte aligned.
Messages contain:
- the process and thread ID of the sender. The thread ID is optional.
- flags indicating if this message is a reply, if it contains a memory operation, and if it should be deleted from the thread's receive queue yet
- the number of blocks total used by the message
- a unique message ID
- optionally, the unique message ID this message is in response to
- optionally, a memory loan or move that can be accepted by the receiver
- some amount of data

Messages can be sent to a process' designated receiver thread without knowing the thread ID by leaving the thread ID unspecified.

The kernel must store messages that are in transit, having been sent but not yet received.
To do this, the kernel loans, for each thread, a region of memory to hold received messages for that thread.
The process does not actually need to know about this loan, because it receives the necessary slices from the `receive` system call.
Threads must mark the messages as read/deletable after they are done with them so the kernel can reuse the space.

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

(Notational note: we use the `*mut [T]` notation to indicate that there is a `*mut T` that actually has more than one `T` in an array.)

#### `send`
The `send` system call allows a process to send a message to another process.
The kernel will inspect the message header and automatically process any associated memory operations while it generates the header on the receiver side.
The message body will be copied to the receiver.

##### Arguments
| Name       | Type                 | Notes                            |
+------------+----------------------+----------------------------------+
| `msg`      | `*const [MessageBlock]`| Pointer to the start of memory in user space that contains the message. |
| `len`      | u8                   | Number of blocks the message contains total. |
| `msg_id`   | `*mut MessageId`     | If non-null, writes the unique ID of this message if it is successfully sent. Otherwise the value is preserved. |
| `flags`    | bitflag              | Options flags for this system call (see the `Flags` section). |

##### Flags
The `send` call accepts the following flags:
| Name           | Description                              |
+----------------+------------------------------------------+

##### Errors
- `NotFound`: the process/thread ID was unknown to the system.
- `BadFormat`: the message header was incorrectly formatted.
- `NoSpace`: the receiving process has too many queued messages and cannot receive the message.
- `InvalidLength`: the length of the message is invalid, i.e. `len` is not in `1..=16`.
- `InvalidFlags`: an unknown or invalid flag combination was passed.
- `InvalidPointer`: the message pointer was null or invalid.

#### `receive`
The `receive` system call allows a process to receive a message from another process.
By default, loans are automatically applied if attached, and their details relative to the receiver will be present in the received message header.
The pointer returned by `receive` is valid until the message is marked for deletion.

This call will by default set the thread to a waiting state if there are no messages.
The thread will resume its running state when it receives a message.
This can be disabled with the `Nonblocking` flag, which will return `WouldBlock` as an error instead if there are no messages.

##### Arguments
| Name       | Type                 | Notes                            |
+------------+----------------------+----------------------------------+
| `msg`      | `*mut *mut [MessageBlock]`| Writes the pointer to the received message data here. |
| `len`      | `*mut u8`            | Writes the number of blocks the message contains total. |
| `flags`    | bitflag              | Options flags for this system call (see the `Flags` section). |

##### Flags
The `receive` call accepts the following flags:
| Name           | Description                              |
+----------------+------------------------------------------+
| `Nonblocking`  | Causes the kernel to return the `WouldBlock` error if there are no messages instead of pausing the thread. |
| `DenyMemoryTransfer` | Causes the kernel to ignore any memory operations contained in the received message. |

##### Errors
- `WouldBlock`: returned in non-blocking mode if there are no messages to receive.
- `InvalidFlags`: an unknown or invalid flag combination was passed.
- `InvalidPointer`: the message pointer or length pointer was null or invalid.


#### `current_process_id`
#### `current_thread_id`
#### `current_supervisor_id`
#### `spawn_process`
#### `exit_current_process`
#### `kill_process`
#### `spawn_thread`
#### `allocate_heap_pages`
#### `free_heap_pages`
#### `driver_request_memory_region`
#### `driver_register_interrupt`

#### Error Codes
| Number | Cause                                            |
+--------+--------------------------------------------------+

### TODO: Interrupts
### TODO: Debug Logging

## Implementation Thoughts
This section is just some thoughts about implementation details. Things may or may not turn out like this.

### Messages
Messages should be short enough to be copied in a few instructions.
A single `LD1` instruction can load up to 64 bytes (using four actual loads), which motivates the message block size.


### Device Driver Servers
A typical device driver server would be spawned as a 'driver' type process by the `init` process.
The driver would first request loans from the kernel or its lower-level driver, set up interrupts with the kernel, and then initialize the device.
The driver then listens for requests from clients, and handles them using the device. Ideally most actual data transfers happen using loans.
