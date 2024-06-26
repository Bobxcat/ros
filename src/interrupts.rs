use pc_keyboard::{DecodedKey, KeyCode, Keyboard, ScancodeSet1};
use pic8259::ChainedPics;
use spin::{Lazy, Mutex};
use x86_64::{
    instructions::port::PortReadOnly,
    structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode},
};

use crate::{gdt, halt_loop, vga_buffer::VgaWriter, vga_print, vga_println};

static IDT: Lazy<InterruptDescriptorTable> = Lazy::new(|| {
    let mut idt = InterruptDescriptorTable::new();
    idt.breakpoint.set_handler_fn(breakpoint_handler);
    unsafe {
        idt.double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
    }
    idt.page_fault.set_handler_fn(page_fault_handler);
    idt[InterruptIndex::Timer.as_u8()].set_handler_fn(timer_interrupt_handler);
    idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_interrupt_handler);
    idt
});

pub fn init_idt() {
    IDT.load();
}

// Exceptions

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    vga_println!("EXCEPTION: Breakpoint\n{stack_frame:#?}");
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _err_code: u64,
) -> ! {
    panic!("EXCEPTION: Double Fault\n{stack_frame:#?}");
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    use x86_64::registers::control::Cr2;

    vga_println!("EXCEPTION: PAGE FAULT");
    vga_println!("Accessed Address: {:?}", Cr2::read());
    vga_println!("Error Code: {:?}", error_code);
    vga_println!("{:#?}", stack_frame);
    halt_loop();
}

// External Interrupts

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // vga_print!(".");

    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Timer.as_u8())
    }
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    static KEYBOARD: Mutex<Keyboard<pc_keyboard::layouts::Us104Key, ScancodeSet1>> =
        Mutex::new(Keyboard::new(
            ScancodeSet1::new(),
            pc_keyboard::layouts::Us104Key,
            pc_keyboard::HandleControl::Ignore,
        ));

    let mut keyboard = KEYBOARD.lock();
    let mut port = PortReadOnly::new(0x60);

    let scancode: u8 = unsafe { port.read() };
    if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
        if let Some(key) = keyboard.process_keyevent(key_event) {
            match key {
                DecodedKey::Unicode(character) => {
                    vga_print!("{}", character);
                }
                DecodedKey::RawKey(key) => match key {
                    KeyCode::LShift | KeyCode::RShift => (),
                    _ => vga_print!("{key:?}"),
                },
            }
        }
    }

    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8())
    }
}

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

pub fn init_pics() {
    unsafe { PICS.lock().initialize() }
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard = PIC_1_OFFSET + 1,
}

impl InterruptIndex {
    #[allow(unused)]
    fn as_u8(self) -> u8 {
        self.into()
    }

    #[allow(unused)]
    fn as_usize(self) -> usize {
        self.into()
    }
}

impl From<InterruptIndex> for u8 {
    fn from(value: InterruptIndex) -> Self {
        value as u8
    }
}

impl From<InterruptIndex> for usize {
    fn from(value: InterruptIndex) -> Self {
        usize::from(u8::from(value))
    }
}
