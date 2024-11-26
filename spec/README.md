# Cavern: Building a modern operating system for no good reason
This spec attempts to describe the design of the Cavern.
This document describes the abstract goals for the project.
The overall goal of the Cavern project is to build an operating system for fun.
Fun is the most important aspect, because this is a hobby project.
Ideally, it is also educational to some degree.

The name "Cavern" is inspired by hobby tunneling, in which people dig tunnels for no reason but the pure enjoyment of digging.
Like digging a tunnel, building an OS is hard work with lots of stressful decisions and trade-offs, but with the right mindset it can be enjoyable for the committed.

## Tools
- Rust
- [U-boot (bootloader)](https://source.denx.de/u-boot/u-boot)

## Architectural Requirements
The architecture of Cavern needs to be foremost hackable, maintainable, robust and extensible.

- **Hackability**

    The architecture should support using fun, exotic technologies and patterns.
    Making the code itself fun and interesting makes it more fun to hack on.
    This of course must be tempered by the needs of maintainability.
    The system should also aim to minimize surprises when composing elements or writing new code.

    Another thing that aids hackability is good documentation, which makes it easier to stay focused on one component without having to remember everything about other components you are interfacing with.
    The best documentation is descriptive names (of all definitions) and descriptive types.
    Additional documentation should strive to go beyond the names and types of things, with the goal being that it is effectively redundent.

    Finally code that is small tends to be more hackable -- modules should aim to be as big as they need to be and no larger. Trying to hack in 1 million lines of code is a lot less fun than hacking on 1000 lines of code.

- **Maintainability**

    Since this is a pointless hobby project, it will mostly be worked on in fits and starts during weekends and evenings.
    If the architecture is unmaintainable, the project will quickly become untenable due to the amount of work required to just get oriented again in the code base after a long break.
    Additionally, a maintainable architecture makes it easier to have fun without spending time doing stressful refactors to fix prior mistakes.

- **Robustness**

    Debugging an operating system can be very difficult and stressful due to the lack of information and tooling.
    Having a robust system that makes it easy to understand and track errors makes debugging more fun and enjoyable.
    Robust systems are also more predictable, making it easier to hack on the system.

- **Extensibility**

    The system must be extensible so that building new features is easy and you so that you don't have to worry about breaking the rest of the system.
    Making the system extensible also makes it easier to experiment with new components.

- **Performance**

    Exceptional performance is not a goal of this project, but trade-offs that greatly reduce performance of the system should be avoided.


### Design Goals
#### Separate Policy and Mechanism
Policy code describes what is to be done.
Policy code is platonically pure, although it does not need to be in practice.
Policy code should free of `unsafe` code.

Mechanism code notates how to accomplish the results of a policy.
Mechanism code is mostly side effects that effect the policy on the system.
All `unsafe` code should be isolated in mechanism code whenever possible.

By keeping these separate whenever possible, we make the system much easier to test, we make it far easier to reuse components, and we make it easier to track and debug side-effects.
Additionally this separation makes the system more maintainable, because often fixes or changes are isolated to either policy or mechanism, and so if isolated we can isolate the amount of code changed and potentially broken.
This design creates a quasi-pure core that is much easier to test and debug, and makes it easier to reuse abstract effects that have been created by agglomerating primitive effects to execute the same or similar policies.

#### Maximize Reusability via Composition
Rust is a language that encourages composition rather than inheritance.
This means that components must be compostable to maximize reuse.
More reusable code yields more extensible and maintainable code.

#### Avoid Duplicate Efforts
Code duplication is bad and confusing. It is better to have one well-crafted abstraction than twenty nearly identical components.

#### Make Everything Explicit
Explicit rules, conditions and effects are much easier to understand and debug than if they are implicit.
This means that the type system should be used maximally to make explicit things like bitfields, strongly packed structs, pointer magic, etc.

#### Error Handling is a First Class Concern
Both the error code path and the normal code path warrant the same care and effort, both should be robust and complete.
Errors should be descriptive and accessible for both humans *and* computers.
Panicking should be reserved for extreme conditions where continuing to execute is impossible or unsafe.

## Platform Requirements
The overarching idea is to attempt to support the most modern, most widely available hardware.
Legacy interfaces should be avoided if possible, except where it would greatly compromise being widely supported.

### Board
The primary target platform is Aarch64/ARMv8 devices, most importantly the QEMU `virt` board.
Additionally, support for the Raspberry Pi would be very nice.
The choice to support ARM is largely due to the large number of available devices and the modern nature of the architecture.
Supporting Aarch64 means we can ignore legacy complications that might be found on other platforms like x86-64.

### Device Interfaces
In order to support the most devices with the least effort and least system complexity, only the most popular modern hardware interfaces should be supported.
Choosing to implement interfaces for only the most common hardware allows us to maximize the return on effort expended.

Interfaces that should be implemented:
- PCIe

    Supporting the PCIe bus allows us to support a huge set of devices relatively easily, and it is easily the highest performance bus in common use.

- NVMe (storage)
- xHCI (USB devices)
- SPI (SD cards)

    SPI is very common on smaller single board computers. Unfortunately as far as I know, QEMU doesn't emulate it well/at all, so this requires real hardware to test.

### Protocol Support
It is necessary to support a number of protocols, like file systems and networking protocols. Ideally these can be kept to a minimal set of modern options.

## Specific Misc. Implementation Goals
A few miscellaneous goals for implementation that apply broadly or were such an oof in the previous attempt they are top of mind:

- Use identity mapping in kernel space i.e. kernel pointers are physical pointers. A lot of the complexity in the previous attempt was because the kernel had its own page tables that had to be managed.
- Implement SMP as soon as possible
- Leverage the borrow checker to provide memory safety
- Be fearlessly concurrent via Rust guarantees, but without compromising on performance.
- Do more type level programming, but not for anything that should be parameterized at runtime.
- Aggressively unit test everything
- Use CI from the beginning
- Use `tracing` instead of `log`
- Use `snafu` in an organized way, with a single top level `Error` enum.
- Try to stay out of the `async` hole
