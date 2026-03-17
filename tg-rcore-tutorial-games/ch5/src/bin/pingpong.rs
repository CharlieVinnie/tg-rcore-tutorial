#![no_std]
#![no_main]

use core::sync::atomic::{AtomicBool, Ordering};
use games_lib::{OpenFlags, fork, get_time, ioctl, mmap, open, read, sched_yield};
use games_lib::constants::*;

extern crate games_lib;

// Import our new graphics library
use games_lib::display::Display;

// --- INPUT & CONSTANTS ---
const EV_KEY: u16 = 1;
const KEY_W: u16 = 17;
const KEY_S: u16 = 31;
const KEY_UP: u16 = 103;
const KEY_DOWN: u16 = 108;
const KEY_Q: u16 = 16;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub timestamp_usec: u64,
    pub event_type: u16,
    pub code: u16,
    pub value: u32,
}

const PADDLE_W: i32 = 15;
const PADDLE_H: i32 = 60;
const BALL_SIZE: i32 = 10;
const PADDLE_SPEED: i32 = 5;

const COLOR_BG: (u8, u8, u8) = (20, 20, 20);
const COLOR_P1: (u8, u8, u8) = (255, 50, 50);
const COLOR_P2: (u8, u8, u8) = (50, 150, 255);
const COLOR_BALL: (u8, u8, u8) = (255, 255, 255);
const COLOR_TEXT: (u8, u8, u8) = (150, 150, 150); // Slightly dimmer text so it's not distracting

// --- SHARED STATE ---
#[repr(C)]
struct GameState {
    lock: AtomicBool,
    running: bool,
    paddle_a_y: i32, paddle_b_y: i32,
    p1_up: bool, p1_down: bool, p2_up: bool, p2_down: bool,
    p1_up_latch: bool, p1_down_latch: bool, p2_up_latch: bool, p2_down_latch: bool,
    ball_x: i32, ball_y: i32,
    ball_vx: i32, ball_vy: i32,
    score_a: u32, score_b: u32,
}

impl GameState {
    fn lock(&self) { while self.lock.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() { sched_yield(); } }
    fn unlock(&self) { self.lock.store(false, Ordering::Release); }
}

// --- CHILD PROCESSES ---
fn player_a_process(state_ptr: *mut GameState, screen_h: i32) {
    let state = unsafe { &mut *state_ptr };
    let mut last_update = get_time();
    loop {
        let now = get_time();
        if now - last_update > 16 {
            state.lock();
            if !state.running { state.unlock(); break; }
            if (state.p1_up || state.p1_up_latch) && !state.p1_down { state.paddle_a_y = (state.paddle_a_y - PADDLE_SPEED).max(80); state.p1_up_latch = false; }
            else if (state.p1_down || state.p1_down_latch) && !state.p1_up { state.paddle_a_y = (state.paddle_a_y + PADDLE_SPEED).min(screen_h - 40 - PADDLE_H); state.p1_down_latch = false; }
            state.unlock();
            last_update = now;
        }
        sched_yield();
    }
}

fn player_b_process(state_ptr: *mut GameState, screen_h: i32) {
    let state = unsafe { &mut *state_ptr };
    let mut last_update = get_time();
    loop {
        let now = get_time();
        if now - last_update > 16 {
            state.lock();
            if !state.running { state.unlock(); break; }
            if (state.p2_up || state.p2_up_latch) && !state.p2_down { state.paddle_b_y = (state.paddle_b_y - PADDLE_SPEED).max(80); state.p2_up_latch = false; }
            else if (state.p2_down || state.p2_down_latch) && !state.p2_up { state.paddle_b_y = (state.paddle_b_y + PADDLE_SPEED).min(screen_h - 40 - PADDLE_H); state.p2_down_latch = false; }
            state.unlock();
            last_update = now;
        }
        sched_yield();
    }
}

// --- MAIN PROCESS ---
#[unsafe(no_mangle)]
fn main() -> i32 {
    let fb_fd = open("/dev/fb0\0", OpenFlags::RDWR);
    if fb_fd < 0 { return -1; }

    let mut res: (u32, u32) = (0, 0);
    ioctl(fb_fd as usize, FB_GET_RESOLUTION, &mut res as *mut _ as usize);
    let screen_w = res.0 as i32;
    let screen_h = res.1 as i32;

    let fb_base = mmap(0, (screen_w * screen_h * 4) as usize, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, fb_fd as usize, 0);
    if fb_base < 0 { return -1; }

    let display = Display { ptr: fb_base as *mut u32, w: screen_w as usize, h: screen_h as usize };

    let shm_size = core::mem::size_of::<GameState>();
    let shm_ptr = mmap(0, shm_size, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, usize::MAX, 0) as *mut GameState;
    if shm_ptr.is_null() { return -1; }

    unsafe {
        *shm_ptr = GameState {
            lock: AtomicBool::new(false), running: true,
            paddle_a_y: screen_h / 2 - PADDLE_H / 2, paddle_b_y: screen_h / 2 - PADDLE_H / 2,
            p1_up: false, p1_down: false, p2_up: false, p2_down: false,
            p1_up_latch: false, p1_down_latch: false, p2_up_latch: false, p2_down_latch: false,
            ball_x: screen_w / 2, ball_y: screen_h / 2,
            ball_vx: 4, ball_vy: 4,
            score_a: 0, score_b: 0,
        };
    }

    if fork() == 0 { player_a_process(shm_ptr, screen_h); return 0; }
    if fork() == 0 { player_b_process(shm_ptr, screen_h); return 0; }

    let kb_fd = open("/dev/input0\0", OpenFlags::RDWR);
    if kb_fd < 0 { return -1; }

    let state = unsafe { &mut *shm_ptr };
    
    // --- 1. NEW INITIAL UI SETUP ---
    display.fill_rect(0, 0, screen_w as usize, screen_h as usize, COLOR_BG);
    
    // Draw the top and bottom court boundaries to separate UI from gameplay
    display.fill_rect(0, 78, screen_w as usize, 2, (100, 100, 100)); // Top Line
    display.fill_rect(0, (screen_h - 40) as usize, screen_w as usize, 2, (100, 100, 100)); // Bottom Line

    // Draw Controls (Pushed to the bottom margin, fixed overflow)
    display.draw_string(20, (screen_h - 30) as usize, "P1: W/S", COLOR_P1, 2);
    display.draw_string((screen_w - 200) as usize, (screen_h - 30) as usize, "P2: UP/DOWN", COLOR_P2, 2);
    display.draw_string((screen_w / 2 - 50) as usize, (screen_h - 30) as usize, "Q: QUIT", COLOR_TEXT, 2);
    
    // Draw initial scores (Top margin)
    let mut old_score_a = 0;
    let mut old_score_b = 0;
    display.draw_number(screen_w as usize / 4, 30, old_score_a, COLOR_TEXT, 4);
    display.draw_number(screen_w as usize * 3 / 4, 30, old_score_b, COLOR_TEXT, 4);

    let mut last_update = get_time();
    let mut old_pa = screen_h / 2 - PADDLE_H / 2;
    let mut old_pb = screen_h / 2 - PADDLE_H / 2;
    let mut old_bx = screen_w / 2;
    let mut old_by = screen_h / 2;

    loop {
        // --- Input Routing ---
        loop {
            let mut event = InputEvent { timestamp_usec: 0, event_type: 0, code: 0, value: 0 };
            let event_slice = core::ptr::slice_from_raw_parts_mut(&mut event as *mut _ as *mut u8, core::mem::size_of::<InputEvent>());
            let ret = unsafe { read(kb_fd as usize, &mut *event_slice) };
            if ret <= 0 { break; }
            if event.event_type == EV_KEY {
                state.lock();
                match event.code {
                    KEY_W => { if event.value == 1 { state.p1_up = true; state.p1_up_latch = true; } else if event.value == 0 { state.p1_up = false; } }
                    KEY_S => { if event.value == 1 { state.p1_down = true; state.p1_down_latch = true; } else if event.value == 0 { state.p1_down = false; } }
                    KEY_UP => { if event.value == 1 { state.p2_up = true; state.p2_up_latch = true; } else if event.value == 0 { state.p2_up = false; } }
                    KEY_DOWN => { if event.value == 1 { state.p2_down = true; state.p2_down_latch = true; } else if event.value == 0 { state.p2_down = false; } }
                    KEY_Q => { state.running = false; state.unlock(); return 0; }
                    _ => {}
                }
                state.unlock();
            }
        }

        // --- Physics Engine ---
        if get_time() - last_update > 16 {
            state.lock();
            state.ball_x += state.ball_vx;
            state.ball_y += state.ball_vy;

            if state.ball_y <= 80 || state.ball_y + BALL_SIZE >= screen_h - 40 { 
                state.ball_vy = -state.ball_vy; 
                
                // Nudge ball out of the wall to prevent it getting stuck
                if state.ball_y <= 80 { state.ball_y = 80; }
                if state.ball_y + BALL_SIZE >= screen_h - 40 { state.ball_y = screen_h - 40 - BALL_SIZE; }
            }

            if state.ball_x <= 30 + PADDLE_W && state.ball_y + BALL_SIZE >= state.paddle_a_y && state.ball_y <= state.paddle_a_y + PADDLE_H {
                state.ball_vx = -state.ball_vx; state.ball_x = 30 + PADDLE_W; 
            }
            if state.ball_x + BALL_SIZE >= screen_w - 30 - PADDLE_W && state.ball_y + BALL_SIZE >= state.paddle_b_y && state.ball_y <= state.paddle_b_y + PADDLE_H {
                state.ball_vx = -state.ball_vx; state.ball_x = screen_w - 30 - PADDLE_W - BALL_SIZE;
            }

            if state.ball_x < 0 { state.score_b += 1; state.ball_x = screen_w / 2; state.ball_y = screen_h / 2; state.ball_vx = 4; } 
            else if state.ball_x > screen_w { state.score_a += 1; state.ball_x = screen_w / 2; state.ball_y = screen_h / 2; state.ball_vx = -4; }

            let draw_pa = state.paddle_a_y; let draw_pb = state.paddle_b_y;
            let draw_bx = state.ball_x; let draw_by = state.ball_y;
            let new_score_a = state.score_a; let new_score_b = state.score_b;
            state.unlock();
            last_update = get_time();

            // --- Dirty Rectangles Rendering ---
            display.fill_rect(30, old_pa as usize, PADDLE_W as usize, PADDLE_H as usize, COLOR_BG);
            display.fill_rect((screen_w - 30 - PADDLE_W) as usize, old_pb as usize, PADDLE_W as usize, PADDLE_H as usize, COLOR_BG);
            display.fill_rect(old_bx as usize, old_by as usize, BALL_SIZE as usize, BALL_SIZE as usize, COLOR_BG);

            // Dirty rectangle for UI: Only erase and redraw the score if it changed!
            if new_score_a != old_score_a {
                display.fill_rect(screen_w as usize / 4, 30, 8 * 4 * 3, 8 * 4, COLOR_BG); // Erase up to 3 digits
                display.draw_number(screen_w as usize / 4, 30, new_score_a, COLOR_TEXT, 4);
                old_score_a = new_score_a;
            }
            if new_score_b != old_score_b {
                display.fill_rect(screen_w as usize * 3 / 4, 30, 8 * 4 * 3, 8 * 4, COLOR_BG);
                display.draw_number(screen_w as usize * 3 / 4, 30, new_score_b, COLOR_TEXT, 4);
                old_score_b = new_score_b;
            }

            for y in (0..screen_h).step_by(20) { display.fill_rect((screen_w / 2 - 2) as usize, y as usize, 4, 10, (100, 100, 100)); }

            display.fill_rect(30, draw_pa as usize, PADDLE_W as usize, PADDLE_H as usize, COLOR_P1);
            display.fill_rect((screen_w - 30 - PADDLE_W) as usize, draw_pb as usize, PADDLE_W as usize, PADDLE_H as usize, COLOR_P2);
            display.fill_rect(draw_bx as usize, draw_by as usize, BALL_SIZE as usize, BALL_SIZE as usize, COLOR_BALL);

            old_pa = draw_pa; old_pb = draw_pb; old_bx = draw_bx; old_by = draw_by;
            ioctl(fb_fd as usize, FB_FLUSH, 0);
        }
    }
}