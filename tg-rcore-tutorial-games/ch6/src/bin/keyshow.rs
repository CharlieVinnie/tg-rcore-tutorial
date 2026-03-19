#![no_std]
#![no_main]

use games_lib::{OpenFlags, open, println, read};

extern crate games_lib;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub timestamp: u64,
    pub event_type: u16,
    pub code: u16,
    pub value: u32,
}

#[unsafe(no_mangle)]
fn main() -> i32 {
    let kb = open("/dev/input0", OpenFlags::RDWR);

    println!("Press Esc to exit >w<");

    loop {
        let mut event = InputEvent { timestamp: 0, event_type: 0, code: 0, value: 0 };
        let event_slice = core::ptr::slice_from_raw_parts_mut(&mut event as *mut InputEvent as *mut u8, core::mem::size_of::<InputEvent>());
        let ret = unsafe { read(kb as usize, &mut *event_slice) };
        if ret != 0 {
            println!("{:?}", event);
            if event.code == 1 && event.value == 0 {
                break;
            }
        }
    }

    0
}