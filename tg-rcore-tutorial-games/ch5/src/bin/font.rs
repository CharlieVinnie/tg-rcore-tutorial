#![no_std]
#![no_main]

use games_lib::{OpenFlags, ioctl, mmap, open, read};
use games_lib::constants::*;

extern crate games_lib;
use games_lib::display::Display;

// Input tracking for quitting
const EV_KEY: u16 = 1;
const KEY_Q: u16 = 16;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub timestamp_usec: u64,
    pub event_type: u16,
    pub code: u16,
    pub value: u32,
}

#[unsafe(no_mangle)]
fn main() -> i32 {
    let fb_fd = open("/dev/fb0\0", OpenFlags::RDWR);
    if fb_fd < 0 { return -1; }

    let mut res: (u32, u32) = (0, 0);
    ioctl(fb_fd as usize, FB_GET_RESOLUTION, &mut res as *mut _ as usize);
    let screen_w = res.0 as usize;
    let screen_h = res.1 as usize;

    let fb_base = mmap(0, screen_w * screen_h * 4, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, fb_fd as usize, 0);
    if fb_base < 0 { return -1; }

    let display = Display { ptr: fb_base as *mut u32, w: screen_w, h: screen_h };

    // Clear Screen
    display.fill_rect(0, 0, screen_w, screen_h, (20, 20, 30));
    
    // Draw Title
    display.draw_string(20, 20, "ASCII FONT DEBUGGER", (255, 255, 0), 2);
    display.draw_string(20, 50, "Press Q to exit", (150, 150, 150), 1);

    // --- GRID LAYOUT LOGIC ---
    let scale = 3;
    let chars_per_row = 16;
    let spacing_x = 40; 
    let spacing_y = 50; 
    let start_x = 40;
    let start_y = 100;

    // Loop through all 95 characters (32 to 126)
    for i in 32..=126 {
        let index = i - 32;
        let row = index / chars_per_row;
        let col = index % chars_per_row;
        
        let cx = start_x + (col * spacing_x) as usize;
        let cy = start_y + (row * spacing_y) as usize;

        // Draw the character
        display.draw_char(cx, cy, i as u8, (200, 255, 200), scale);
    }

    ioctl(fb_fd as usize, FB_FLUSH, 0);

    // Open input and wait for 'Q' to exit
    let kb_fd = open("/dev/input0\0", OpenFlags::RDWR);
    if kb_fd < 0 { return -1; }

    loop {
        let mut event = InputEvent { timestamp_usec: 0, event_type: 0, code: 0, value: 0 };
        let event_slice = core::ptr::slice_from_raw_parts_mut(&mut event as *mut _ as *mut u8, core::mem::size_of::<InputEvent>());
        let ret = unsafe { read(kb_fd as usize, &mut *event_slice) };
        
        if ret > 0 && event.event_type == EV_KEY && event.value == 1 {
            if event.code == KEY_Q {
                return 0; // Exit program safely
            }
        }
    }
}