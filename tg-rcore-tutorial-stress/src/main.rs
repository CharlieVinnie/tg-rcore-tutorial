#![no_std]
#![no_main]

extern crate tg_framework;

#[macro_use]
extern crate tg_console;

use tg_console::{Console, init_console};
use tg_sbi::console_putchar;
use spin::Mutex;

const BUFFER_SIZE: usize = 4096;

struct ConsoleOutput {
    buffer: [u8; BUFFER_SIZE],
    pos: usize,
}
static OUTPUT: Mutex<ConsoleOutput> = Mutex::new(ConsoleOutput { buffer: [0; BUFFER_SIZE], pos: 0 });

// --- Dummy Console ---
struct DummyConsole;

impl Console for DummyConsole {
    fn put_char(&self, c: u8) {
        console_putchar(c);
        let mut out = OUTPUT.lock();
        let p = out.pos;
        if p < BUFFER_SIZE {
            out.buffer[p] = c;
            out.pos += 1;
        }
    }
}

static DUMMY_CONSOLE: DummyConsole = DummyConsole;

#[unsafe(no_mangle)]
pub extern "C" fn rust_main_prelude() {
    init_console(&DUMMY_CONSOLE);
    println!("Hart 0 finished prelude.");
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_main_execute(hartid: usize) {
    // Stress test println!
    for i in 0..10 {
        println!("Stress test from hart {}, iteration {}", hartid, i);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_main_epilogue() -> ! {
    let mut valid_lines = 0;
    {
        let out = OUTPUT.lock();
        let s = core::str::from_utf8(&out.buffer[..out.pos]).unwrap();
        // Skip the prelude line
        let lines = s.split('\n').filter(|l| !l.is_empty() && !l.contains("prelude"));
        for line in lines {
            if !line.starts_with("Stress test from hart ") || !line.contains(", iteration ") {
                panic!("Interleaved output detected: {}", line);
            }
            valid_lines += 1;
        }
    }
    println!("Test completed successfully. Verified {} lines. Shutting down.", valid_lines);
    tg_sbi::shutdown(false);
}
