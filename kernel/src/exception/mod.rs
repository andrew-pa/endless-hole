//! Mechanisms for exception handling

mod handlers;
pub use handlers::install_exception_vector;

/// A stored version of the system registers x0..x31.
// TODO: this doesn't really belong in this module.
#[derive(Default, Copy, Clone, Debug)]
#[repr(C)]
pub struct Registers {
    /// The values of the xN registers in order.
    pub x: [usize; 31],
}

impl Registers {
    /// Create a Registers struct that has the contents of args in the first registers. There can
    /// be up to 8, mirroring the typical ARM64 calling convention.
    pub fn from_args(args: &[usize]) -> Registers {
        assert!(args.len() <= 8);
        let mut regs = Registers::default();
        regs.x[0..args.len()].copy_from_slice(args);
        regs
    }
}

bitfield::bitfield! {
    /// A value in the ESR (Exception Syndrome Register), which indicates the cause of an
    /// exception.
    struct ExceptionSyndromeRegister(u64);
    u8;
    iss2, _: 36, 32;
    u8, into ExceptionClass, ec, _: 31, 26;
    il, _: 25, 25;
    u32, iss, _: 24, 0;
}

struct ExceptionClass(u8);

impl From<u8> for ExceptionClass {
    fn from(value: u8) -> Self {
        Self(value)
    }
}

impl ExceptionClass {
    #[inline]
    fn is_system_call(&self) -> bool {
        self.0 == 0b010101
    }

    #[inline]
    fn is_user_space_data_page_fault(&self) -> bool {
        self.0 == 0b100100
    }

    #[inline]
    fn is_kernel_data_page_fault(&self) -> bool {
        self.0 == 0b100101
    }

    #[inline]
    fn is_data_abort(&self) -> bool {
        self.is_user_space_data_page_fault() || self.is_kernel_data_page_fault()
    }

    #[inline]
    fn is_user_space_code_page_fault(&self) -> bool {
        self.0 == 0b100000
    }
}

impl core::fmt::Debug for ExceptionClass {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "0b{:b}=", self.0)?;
        match self.0 {
            0b000000 => write!(f, "[Misc]"),
            0b000001 => write!(f, "[Trapped WF* instruction]"),
            0b000111 => write!(f, "[Access to SME, SVE, Advanced SIMD or floating-point functionality trapped by CPACR_EL1.FPEN, CPTR_EL2.FPEN, CPTR_EL2.TFP, or CPTR_EL3.TFP control]"),
            0b001010 => write!(f, "[Trapped execution of an LD64B or ST64B* instruction.]"),
            0b001101 => write!(f, "[Branch Target Exception]"),
            0b001110 => write!(f, "[Illegal Execution state]"),
            0b010101 => write!(f, "[SVC instruction]"),
            0b011000 => write!(f, "[Trapped MSR, MRS or System instruction execution in AArch64 state, that is not reported using EC 0b000000, 0b000001, or 0b000111]"),
            0b100000 => write!(f, "[Instruction Abort from a lower Exception level]"),
            0b100001 => write!(f, "[Instruction Abort taken without a change in Exception level]"),
            0b100010 => write!(f, "[PC alignment fault exception]"),
            0b100100 => write!(f, "[Data Abort exception from a lower Exception level]"),
            0b100101 => write!(f, "[Data Abort exception taken without a change in Exception level]"),
            0b100110 => write!(f, "[SP alignment fault exception]"),
            _ => write!(f, "[Unknown]")
        }
    }
}

fn data_abort_dfsc_description(code: u8) -> &'static str {
    match code {
        0b000000 => "Address size fault, level 0 of translation or translation table base register",
        0b000001 => "Address size fault, level 1",
        0b000010 => "Address size fault, level 2",
        0b000011 => "Address size fault, level 3",
        0b000100 => "Translation fault, level 0",
        0b000101 => "Translation fault, level 1",
        0b000110 => "Translation fault, level 2",
        0b000111 => "Translation fault, level 3",
        0b001000 => "Access flag fault, level 0",
        0b001001 => "Access flag fault, level 1",
        0b001010 => "Access flag fault, level 2",
        0b001011 => "Access flag fault, level 3",
        0b001100 => "Permission fault, level 0",
        0b001101 => "Permission fault, level 1",
        0b001110 => "Permission fault, level 2",
        0b001111 => "Permission fault, level 3",
        0b010000 => "Synchronous External abort, not on translation table walk or hardware update of translation table",
        0b010001 => "Synchronous Tag Check Fault",
        0b010011 => "Synchronous External abort on translation table walk or hardware update of translation table, level -1",
        0b010100 => "Synchronous External abort on translation table walk or hardware update of translation table, level 0",
        0b010101 => "Synchronous External abort on translation table walk or hardware update of translation table, level 1",
        0b010110 => "Synchronous External abort on translation table walk or hardware update of translation table, level 2",
        0b010111 => "Synchronous External abort on translation table walk or hardware update of translation table, level 3",
        0b011000 => "Synchronous parity or ECC error on memory access, not on translation table walk",
        0b011011 => "Synchronous parity or ECC error on memory access on translation table walk or hardware update of translation table, level -1",
        0b011100 => "Synchronous parity or ECC error on memory access on translation table walk or hardware update of translation table, level 0",
        0b011101 => "Synchronous parity or ECC error on memory access on translation table walk or hardware update of translation table, level 1",
        0b011110 => "Synchronous parity or ECC error on memory access on translation table walk or hardware update of translation table, level 2",
        0b011111 => "Synchronous parity or ECC error on memory access on translation table walk or hardware update of translation table, level 3",
        0b100001 => "Alignment fault",
        0b100011 => "Granule Protection Fault on translation table walk or hardware update of translation table, level -1",
        0b100100 => "Granule Protection Fault on translation table walk or hardware update of translation table, level 0",
        0b100101 => "Granule Protection Fault on translation table walk or hardware update of translation table, level 1",
        0b100110 => "Granule Protection Fault on translation table walk or hardware update of translation table, level 2",
        0b100111 => "Granule Protection Fault on translation table walk or hardware update of translation table, level 3",
        0b101000 => "Granule Protection Fault, not on translation table walk or hardware update of translation table",
        0b101001 => "Address size fault, level -1",
        0b101011 => "Translation fault, level -1",
        0b110000 => "TLB conflict abort",
        0b110001 => "Unsupported atomic hardware update fault",
        0b110100 => "IMPLEMENTATION DEFINED fault (Lockdown)",
        0b110101 => "IMPLEMENTATION DEFINED fault (Unsupported Exclusive or Atomic access)",
        _ => "?",
    }
}

impl core::fmt::Display for ExceptionSyndromeRegister {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ESR")
            .field("ISS2", &format_args!("0x{:x}", self.iss2()))
            .field("EC", &self.ec())
            .field("IL", &self.il())
            // TODO: the ISS field could be decoded further
            .field(
                "ISS",
                &format_args!(
                    "0x{:x}=0b{:b}=[{}]",
                    self.iss(),
                    self.iss(),
                    if self.ec().is_data_abort() {
                        data_abort_dfsc_description((self.iss() & 0b11_1111) as u8)
                    } else {
                        ""
                    }
                ),
            )
            .finish()
    }
}
