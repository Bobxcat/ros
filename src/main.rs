#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(ros::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::panic::PanicInfo;
use ros::{halt_loop, serial_println, vga_print, vga_println};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    ros::init();

    vga_println!("Hello VGA!");
    vga_println!("Ask me a question and I will answer");
    serial_println!("Hello Serial!");

    vga_print!("  > ");

    #[cfg(test)]
    test_main();

    halt_loop();
}

/// This function is called on panic.
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    vga_println!("{}", info);

    halt_loop();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    ros::test_panic_handler(info)
}
