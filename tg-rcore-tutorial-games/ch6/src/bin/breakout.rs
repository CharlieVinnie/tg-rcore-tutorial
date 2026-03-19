#![no_std]
#![no_main]

use games_lib::{OpenFlags, close, get_time, ioctl, mmap, open, println, read, write};
use games_lib::constants::*;
use core::mem::size_of;

extern crate games_lib;

use games_lib::display::Display;

const SAVE_PATH: &str = "breakout_save.bin";

// --- Constants ---
const BOARD_W: usize = 10;
const BOARD_H: usize = 8;
const BRICK_W: usize = 76;
const BRICK_H: usize = 20;
const BRICK_GAP: usize = 4;

const PADDLE_W: i32 = 100;
const PADDLE_H: i32 = 10;
const PADDLE_Y_OFFSET: i32 = 50; // From bottom
const PADDLE_SPEED: i32 = 5;

const BALL_SIZE: i32 = 8;
const FIXED_POINT_SHIFT: i32 = 8; // For sub-pixel movement (1 pixel = 256 units)

// Input Events
const EV_KEY: u16 = 1;
const KEY_Q: u16 = 16;
const KEY_A: u16 = 30;
const KEY_D: u16 = 32;
const KEY_F5: u16 = 63;
const KEY_F6: u16 = 64;
const KEY_LEFT: u16 = 105;
const KEY_RIGHT: u16 = 106;
const KEY_SPACE: u16 = 57;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub timestamp_usec: u64,
    pub event_type: u16,
    pub code: u16,
    pub value: u32,
}

// Colors
const COLOR_BG: (u8, u8, u8) = (30, 25, 25);
const COLOR_TEXT: (u8, u8, u8) = (200, 200, 200);
const COLOR_TEXT_GREEN: (u8, u8, u8) = (50, 255, 50);
const COLOR_PADDLE: (u8, u8, u8) = (240, 150, 50);
const COLOR_BALL: (u8, u8, u8) = (255, 255, 255);

const BRICK_COLORS: [(u8, u8, u8); 8] = [
    (230, 60, 60),   // Red
    (240, 130, 40),  // Orange
    (240, 220, 50),  // Yellow
    (80, 200, 80),   // Green
    (60, 200, 200),  // Cyan
    (60, 120, 240),  // Blue
    (160, 80, 220),  // Purple
    (140, 140, 140), // Grey
];

// --- Game State ---
#[repr(C)]
#[derive(Clone, Copy)]
struct GameState {
    bricks: [[u8; BOARD_W]; BOARD_H],
    score: u32,
    lives: u32,
    paddle_x: i32,
    ball_x_fp: i32,
    ball_y_fp: i32,
    ball_dx_fp: i32,
    ball_dy_fp: i32,
    active: bool,
}

impl GameState {
    fn new(screen_w: usize, screen_h: usize) -> Self {
        let mut state = Self {
            bricks: [[0; BOARD_W]; BOARD_H],
            score: 0,
            lives: 3,
            paddle_x: (screen_w as i32 - PADDLE_W) / 2,
            ball_x_fp: 0,
            ball_y_fp: 0,
            ball_dx_fp: 0,
            ball_dy_fp: 0,
            active: false,
        };
        state.reset_bricks();
        state.reset_ball(screen_w, screen_h);
        state
    }

    fn reset_bricks(&mut self) {
        for y in 0..BOARD_H {
            for x in 0..BOARD_W {
                self.bricks[y][x] = (y % 8 + 1) as u8; // 1 to 8 mapped to colors
            }
        }
    }

    fn reset_ball(&mut self, screen_w: usize, screen_h: usize) {
        self.active = false;
        self.paddle_x = (screen_w as i32 - PADDLE_W) / 2;
        self.ball_x_fp = (self.paddle_x + PADDLE_W / 2) << FIXED_POINT_SHIFT;
        self.ball_y_fp = (screen_h as i32 - PADDLE_Y_OFFSET - PADDLE_H - BALL_SIZE) << FIXED_POINT_SHIFT;
        
        // Initial trajectory (up and slightly right)
        self.ball_dx_fp = 3 << (FIXED_POINT_SHIFT - 1); // 1.5 pixels per tick
        self.ball_dy_fp = -(5 << (FIXED_POINT_SHIFT - 1)); // -2.5 pixels per tick
    }
}

// --- Save/Load Helpers ---
fn save_game(state: &GameState) {
    let fd = open(SAVE_PATH, OpenFlags::RDWR | OpenFlags::CREATE | OpenFlags::TRUNC); 
    if fd >= 0 {
        let state_ptr = state as *const _ as *const u8;
        let state_slice = unsafe { core::slice::from_raw_parts(state_ptr, size_of::<GameState>()) };
        write(fd as usize, state_slice);
        close(fd as usize);
        println!("Saved to {}", SAVE_PATH);
    }
}

fn load_game(state: &mut GameState) {
    let fd = open(SAVE_PATH, OpenFlags::RDWR);
    if fd >= 0 {
        let state_ptr = state as *mut _ as *mut u8;
        let state_slice = unsafe { core::slice::from_raw_parts_mut(state_ptr, size_of::<GameState>()) };
        read(fd as usize, state_slice);
        close(fd as usize);
        println!("Loaded from {}", SAVE_PATH);
    }
}

// Manually format and print the FPS onto standard output using backspaces
fn print_fps(fps: u32) {
    let mut buf = [0u8; 32];
    let mut i = 0;
    
    // Print 15 backspaces (\x08) to safely clear the line
    for _ in 0..15 {
        buf[i] = 0x08;
        i += 1;
    }
    
    // Write "FPS: "
    buf[i] = b'F'; i += 1;
    buf[i] = b'P'; i += 1;
    buf[i] = b'S'; i += 1;
    buf[i] = b':'; i += 1;
    buf[i] = b' '; i += 1;

    let mut temp = fps;
    let mut d_count = 0;
    let mut digits = [0u8; 10];
    
    if temp == 0 {
        digits[0] = b'0';
        d_count = 1;
    } else {
        while temp > 0 {
            digits[d_count] = (temp % 10) as u8 + b'0';
            temp /= 10;
            d_count += 1;
        }
    }
    
    while d_count > 0 {
        d_count -= 1;
        buf[i] = digits[d_count];
        i += 1;
    }
    
    // Output trailing spaces to overwrite potential leftover characters
    buf[i] = b' '; i += 1;
    buf[i] = b' '; i += 1;
    
    write(1, &buf[..i]);
}

// --- Main Entry ---
#[unsafe(no_mangle)]
fn main() -> i32 {
    let fb_fd = open("/dev/fb0", OpenFlags::RDWR);
    if fb_fd < 0 { return -1; }

    let mut res: (u32, u32) = (0, 0);
    ioctl(fb_fd as usize, FB_GET_RESOLUTION, &mut res as *mut _ as usize);
    let screen_w = res.0 as usize;
    let screen_h = res.1 as usize;

    println!("Resolution: {} * {}", screen_w, screen_h);
    println!("LEFT/RIGHT = move, SPACE = launch, F5 = save, F6 = load, Q = quit");

    let fb_base = mmap(0, screen_w * screen_h * 4, PROT_READ | PROT_WRITE, MAP_PRIVATE, fb_fd as usize, 0);
    if fb_base == 0 { return -1; }

    let display = Display {
        ptr: fb_base as *mut u32,
        w: screen_w,
        h: screen_h,
    };

    let kb_fd = open("/dev/input0", OpenFlags::RDWR);
    if kb_fd < 0 { return -1; }

    let mut state = GameState::new(screen_w, screen_h);
    let mut last_tick = get_time();
    let tick_rate = 16; // ~60 FPS logic rate

    let play_w = (BOARD_W * BRICK_W) + ((BOARD_W - 1) * BRICK_GAP);
    let offset_x = screen_w.saturating_sub(play_w) / 2;
    let offset_y = 80;

    // --- Dirty Rectangle Tracker ---
    let mut force_full_redraw = true;
    let mut old_paddle_x = state.paddle_x;
    let mut old_ball_x = (state.ball_x_fp >> FIXED_POINT_SHIFT) as usize;
    let mut old_ball_y = (state.ball_y_fp >> FIXED_POINT_SHIFT) as usize;
    let mut old_score = state.score;
    let mut old_lives = state.lives;

    // --- FPS Tracker ---
    let mut fps_counter = 0;
    let mut last_fps_time = get_time();

    loop {
        let now = get_time();

        // Process Input
        while get_time() < now + 2 {
            let mut event = InputEvent { timestamp_usec: 0, event_type: 0, code: 0, value: 0 };
            let event_slice = core::ptr::slice_from_raw_parts_mut(&mut event as *mut _ as *mut u8, size_of::<InputEvent>());
            let ret = unsafe { read(kb_fd as usize, &mut *event_slice) };
            
            if ret > 0 && event.event_type == EV_KEY && (event.value == 1 || event.value == 2) {
                match event.code {
                    KEY_LEFT | KEY_A => {
                        let delta_x = (state.paddle_x - PADDLE_SPEED).max(0) - state.paddle_x;
                        state.paddle_x += delta_x;
                        if !state.active { state.ball_x_fp += delta_x << FIXED_POINT_SHIFT; }
                    }
                    KEY_RIGHT | KEY_D => {
                        let delta_x = (state.paddle_x + PADDLE_SPEED).min(screen_w as i32 - PADDLE_W) - state.paddle_x;
                        state.paddle_x += delta_x;
                        if !state.active { state.ball_x_fp += delta_x << FIXED_POINT_SHIFT; }
                    }
                    KEY_F5 => { save_game(&state); }
                    KEY_F6 => { 
                        load_game(&mut state); 
                        force_full_redraw = true; // Ask renderer to dump the whole buffer again
                    }
                    KEY_Q => { return 0; }
                    KEY_SPACE => {
                        if !state.active && event.value == 1 {
                            state.active = true;
                        }
                    }
                    _ => {}
                }
            }
        }

        // Game Update Tick
        if now - last_tick > tick_rate {
            last_tick = now;

            if state.active {
                // Move Ball
                state.ball_x_fp += state.ball_dx_fp;
                state.ball_y_fp += state.ball_dy_fp;

                let ball_x = state.ball_x_fp >> FIXED_POINT_SHIFT;
                let ball_y = state.ball_y_fp >> FIXED_POINT_SHIFT;

                // Wall Collisions
                if ball_x <= 0 {
                    state.ball_x_fp = 0;
                    state.ball_dx_fp = -state.ball_dx_fp;
                } else if ball_x + BALL_SIZE >= screen_w as i32 {
                    state.ball_x_fp = (screen_w as i32 - BALL_SIZE) << FIXED_POINT_SHIFT;
                    state.ball_dx_fp = -state.ball_dx_fp;
                }

                if ball_y <= 0 {
                    state.ball_y_fp = 0;
                    state.ball_dy_fp = -state.ball_dy_fp;
                } else if ball_y + BALL_SIZE >= screen_h as i32 {
                    // Missed ball (bottom)
                    state.lives = state.lives.saturating_sub(1);
                    if state.lives == 0 {
                        state = GameState::new(screen_w, screen_h);
                    } else {
                        state.reset_ball(screen_w, screen_h);
                    }
                    force_full_redraw = true; 
                    continue; 
                }

                // Paddle Collision
                let paddle_y_actual = screen_h as i32 - PADDLE_Y_OFFSET;
                if ball_y + BALL_SIZE >= paddle_y_actual 
                    && ball_y <= paddle_y_actual + PADDLE_H 
                    && ball_x + BALL_SIZE >= state.paddle_x 
                    && ball_x <= state.paddle_x + PADDLE_W 
                {
                    state.ball_y_fp = (paddle_y_actual - BALL_SIZE) << FIXED_POINT_SHIFT;
                    state.ball_dy_fp = -state.ball_dy_fp;
                    
                    let hit_offset = (ball_x + BALL_SIZE / 2) - (state.paddle_x + PADDLE_W / 2);
                    state.ball_dx_fp += hit_offset * (1 << (FIXED_POINT_SHIFT - 4)); 
                }

                // Brick Collisions
                let mut hit_brick = false;
                for r in 0..BOARD_H {
                    for c in 0..BOARD_W {
                        if state.bricks[r][c] > 0 {
                            let bx = offset_x as i32 + (c * (BRICK_W + BRICK_GAP)) as i32;
                            let by = offset_y as i32 + (r * (BRICK_H + BRICK_GAP)) as i32;

                            // AABB intersection
                            if ball_x + BALL_SIZE >= bx 
                                && ball_x <= bx + BRICK_W as i32 
                                && ball_y + BALL_SIZE >= by 
                                && ball_y <= by + BRICK_H as i32 
                            {
                                state.bricks[r][c] = 0;
                                state.score += 10;
                                hit_brick = true;

                                // Erase brick immediately (if not queuing a full redraw anyway)
                                if !force_full_redraw {
                                    display.fill_rect(bx as usize, by as usize, BRICK_W, BRICK_H, COLOR_BG);
                                }

                                let overlap_left = (ball_x + BALL_SIZE) - bx;
                                let overlap_right = (bx + BRICK_W as i32) - ball_x;
                                let overlap_top = (ball_y + BALL_SIZE) - by;
                                let overlap_bottom = (by + BRICK_H as i32) - ball_y;

                                let min_overlap = overlap_left.min(overlap_right).min(overlap_top).min(overlap_bottom);

                                if min_overlap == overlap_left || min_overlap == overlap_right {
                                    state.ball_dx_fp = -state.ball_dx_fp;
                                } else {
                                    state.ball_dy_fp = -state.ball_dy_fp;
                                }
                                break; 
                            }
                        }
                    }
                    if hit_brick { break; }
                }
            }

            // --- Render Logic ---
            let mut did_full_redraw = false;

            if force_full_redraw {
                // Perform a complete screen clear and UI rebuild
                display.fill_rect(0, 0, screen_w, screen_h, COLOR_BG);

                display.draw_string(20, 20, "SCORE:", COLOR_TEXT, 2);
                display.draw_string(20, 50, "LIVES:", COLOR_TEXT, 2);
                let text_x = screen_w - 300;
                display.draw_string(text_x, 20, "F5:SAVE F6:LOAD", COLOR_TEXT_GREEN, 2);

                for r in 0..BOARD_H {
                    for c in 0..BOARD_W {
                        let brick_val = state.bricks[r][c];
                        if brick_val > 0 {
                            let color = BRICK_COLORS[(brick_val - 1) as usize];
                            let bx = offset_x + c * (BRICK_W + BRICK_GAP);
                            let by = offset_y + r * (BRICK_H + BRICK_GAP);
                            display.fill_rect(bx, by, BRICK_W, BRICK_H, color);
                        }
                    }
                }

                did_full_redraw = true;
                force_full_redraw = false; 
            } else {
                // Dirty Rects: Erase entities at their old coordinates
                display.fill_rect(old_ball_x, old_ball_y, BALL_SIZE as usize, BALL_SIZE as usize, COLOR_BG);

                if old_paddle_x != state.paddle_x {
                    display.fill_rect(
                        old_paddle_x as usize, 
                        screen_h - PADDLE_Y_OFFSET as usize, 
                        PADDLE_W as usize, 
                        PADDLE_H as usize, 
                        COLOR_BG
                    );
                }

                // If UI state changed, blank out the area
                if state.score != old_score {
                    display.fill_rect(130, 20, 100, 20, COLOR_BG);
                }
                if state.lives != old_lives {
                    display.fill_rect(130, 55, 100, 20, COLOR_BG);
                }
            }

            // Paint new states
            let b_x = (state.ball_x_fp >> FIXED_POINT_SHIFT) as usize;
            let b_y = (state.ball_y_fp >> FIXED_POINT_SHIFT) as usize;

            if did_full_redraw || old_paddle_x != state.paddle_x {
                display.fill_rect(
                    state.paddle_x as usize, 
                    screen_h - PADDLE_Y_OFFSET as usize, 
                    PADDLE_W as usize, 
                    PADDLE_H as usize, 
                    COLOR_PADDLE
                );
            }

            display.fill_rect(b_x, b_y, BALL_SIZE as usize, BALL_SIZE as usize, COLOR_BALL);

            if did_full_redraw || state.score != old_score {
                display.draw_number(130, 20, state.score, COLOR_TEXT, 2);
            }

            if did_full_redraw || state.lives != old_lives {
                for i in 0..state.lives {
                    display.fill_rect(130 + (i as usize * 20), 55, 12, 12, COLOR_TEXT);
                }
            }

            // Sync frame trackers
            old_paddle_x = state.paddle_x;
            old_ball_x = b_x;
            old_ball_y = b_y;
            old_score = state.score;
            old_lives = state.lives;

            ioctl(fb_fd as usize, FB_FLUSH, 0);

            // Print FPS logic
            fps_counter += 1;
            if now - last_fps_time >= 1000 {
                print_fps(fps_counter);
                fps_counter = 0;
                last_fps_time = now;
            }
        }
    }
}