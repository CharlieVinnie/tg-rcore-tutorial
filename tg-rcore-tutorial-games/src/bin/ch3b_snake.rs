#![no_std]
#![no_main]

use games_lib::{OpenFlags, get_time, ioctl, mmap, open, println, read};

extern crate games_lib;

const FB_FLUSH: usize = 1;

// Screen and Grid configurations
const SCREEN_W: usize = 800;
const SCREEN_H: usize = 600;
const CELL_SIZE: usize = 20;
const GRID_W: usize = 20;
const GRID_H: usize = 20;
const OFFSET_X: usize = 100;
const OFFSET_Y: usize = 100;

// Maximum snake size (Grid width * Grid height)
const MAX_SNAKE_LEN: usize = GRID_W * GRID_H;

// Standard Linux input event codes
const EV_KEY: u16 = 1;
const KEY_W: u16 = 17;
const KEY_A: u16 = 30;
const KEY_S: u16 = 31;
const KEY_D: u16 = 32;
const KEY_UP: u16 = 103;
const KEY_LEFT: u16 = 105;
const KEY_RIGHT: u16 = 106;
const KEY_DOWN: u16 = 108;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub timestamp_usec: u64,
    pub event_type: u16,
    pub code: u16,
    pub value: u32,
}

// Colors (R, G, B)
const COLOR_BG: (u8, u8, u8) = (35, 30, 30);
const COLOR_GRID: (u8, u8, u8) = (60, 50, 50);
const COLOR_BORDER: (u8, u8, u8) = (255, 255, 255);
const COLOR_SNAKE: (u8, u8, u8) = (80, 200, 80);
const COLOR_FOOD: (u8, u8, u8) = (240, 60, 60);

// Basic Linear Congruential Generator for random food placement
struct Prng {
    state: u32,
}

impl Prng {
    fn next(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(1664525).wrapping_add(1013904223);
        self.state
    }
    fn next_range(&mut self, min: u32, max: u32) -> u32 {
        min + (self.next() % (max - min))
    }
}

// Simple 3x5 pixel font for digits 0-9 to render the score
const DIGITS: [[u8; 15]; 10] = [
    [1,1,1, 1,0,1, 1,0,1, 1,0,1, 1,1,1], // 0
    [0,1,0, 1,1,0, 0,1,0, 0,1,0, 1,1,1], // 1
    [1,1,1, 0,0,1, 1,1,1, 1,0,0, 1,1,1], // 2
    [1,1,1, 0,0,1, 1,1,1, 0,0,1, 1,1,1], // 3
    [1,0,1, 1,0,1, 1,1,1, 0,0,1, 0,0,1], // 4
    [1,1,1, 1,0,0, 1,1,1, 0,0,1, 1,1,1], // 5
    [1,1,1, 1,0,0, 1,1,1, 1,0,1, 1,1,1], // 6
    [1,1,1, 0,0,1, 0,0,1, 0,0,1, 0,0,1], // 7
    [1,1,1, 1,0,1, 1,1,1, 1,0,1, 1,1,1], // 8
    [1,1,1, 1,0,1, 1,1,1, 0,0,1, 1,1,1], // 9
];

#[unsafe(no_mangle)]
fn main() -> i32 {
    let fb_fd = open("/dev/fb0\0", OpenFlags::RDWR);
    if fb_fd < 0 { return -1; }

    let fb_base = mmap(0, SCREEN_W * SCREEN_H * 4, 0, 0, fb_fd as usize, 0);
    if fb_base == 0 { return -1; }
    let fb_ptr = fb_base as *mut u8;

    let kb_fd = open("/dev/input0\0", OpenFlags::RDWR);
    if kb_fd < 0 { return -1; }

    // Game State
    let mut snake = [(0i32, 0i32); MAX_SNAKE_LEN];
    let mut snake_len = 4;
    
    // Initialize snake in the middle
    for i in 0..snake_len {
        snake[i] = ((GRID_W / 2) as i32 - i as i32, (GRID_H / 2) as i32);
    }

    let mut dir = (1i32, 0i32); // Start moving right
    let mut next_dir = dir;
    let mut prng = Prng { state: 1234567 };
    let mut food = (prng.next_range(0, GRID_W as u32) as i32, prng.next_range(0, GRID_H as u32) as i32);
    let mut score: u32 = 0;

    // --- INITIAL RENDER ---
    fill_background(fb_ptr);
    draw_play_area(fb_ptr);
    
    for i in 0..snake_len {
        draw_rect(fb_ptr, OFFSET_X + snake[i].0 as usize * CELL_SIZE, OFFSET_Y + snake[i].1 as usize * CELL_SIZE, CELL_SIZE, CELL_SIZE, COLOR_SNAKE);
    }
    draw_rect(fb_ptr, OFFSET_X + food.0 as usize * CELL_SIZE, OFFSET_Y + food.1 as usize * CELL_SIZE, CELL_SIZE, CELL_SIZE, COLOR_FOOD);
    draw_number(fb_ptr, OFFSET_X, OFFSET_Y - 40, score, COLOR_BORDER);
    
    ioctl(fb_fd as usize, FB_FLUSH, 0);

    // Add an input buffer right before the main loop starts
    let mut input_buffer: [u16; 2] = [0, 0];
    let mut input_count = 0;

    loop {
        let frame_start_time = get_time();

        // --- 2. Process Buffered Input ---
        // Only process ONE queued direction per grid movement
        if input_count > 0 {
            let code = input_buffer[0];
            
            // Shift the queue down
            input_buffer[0] = input_buffer[1];
            input_count -= 1;

            // Notice the condition change: dir.1 == 0 means "if we are moving horizontally, we can only turn vertically"
            match code {
                KEY_UP | KEY_W    if dir.1 == 0 => dir = (0, -1),
                KEY_DOWN | KEY_S  if dir.1 == 0 => dir = (0, 1),
                KEY_LEFT | KEY_A  if dir.0 == 0 => dir = (-1, 0),
                KEY_RIGHT | KEY_D if dir.0 == 0 => dir = (1, 0),
                _ => {} // Invalid move (like trying to reverse), ignored
            }
        }

        // --- 3. Update State ---
        let head = snake[0];
        let new_head = (head.0 + dir.0, head.1 + dir.1);

        // Check Wall Collision
        if new_head.0 < 0 || new_head.0 >= GRID_W as i32 || new_head.1 < 0 || new_head.1 >= GRID_H as i32 {
            break; 
        }

        // Check Self Collision
        let mut self_collision = false;
        for i in 0..snake_len {
            if new_head == snake[i] { self_collision = true; break; }
        }
        if self_collision { break; } 

        let old_tail = snake[snake_len - 1];
        let mut ate_food = false;

        // Check Food
        if new_head == food {
            ate_food = true;
            if snake_len < MAX_SNAKE_LEN {
                snake[snake_len] = old_tail; 
                snake_len += 1;
            }
            score += 1;
            
            food = (prng.next_range(0, GRID_W as u32) as i32, prng.next_range(0, GRID_H as u32) as i32);
            
            draw_rect(fb_ptr, OFFSET_X + food.0 as usize * CELL_SIZE, OFFSET_Y + food.1 as usize * CELL_SIZE, CELL_SIZE, CELL_SIZE, COLOR_FOOD);
            
            fill_rect(fb_ptr, OFFSET_X, OFFSET_Y - 40, 120, 30, COLOR_BG);
            draw_number(fb_ptr, OFFSET_X, OFFSET_Y - 40, score, COLOR_BORDER);
        }

        // Move body
        for i in (1..snake_len).rev() {
            snake[i] = snake[i - 1];
        }
        snake[0] = new_head;

        // --- 4. Render Delta ---
        if !ate_food {
            draw_rect(fb_ptr, OFFSET_X + old_tail.0 as usize * CELL_SIZE, OFFSET_Y + old_tail.1 as usize * CELL_SIZE, CELL_SIZE, CELL_SIZE, COLOR_BG);
        }

        draw_rect(fb_ptr, OFFSET_X + new_head.0 as usize * CELL_SIZE, OFFSET_Y + new_head.1 as usize * CELL_SIZE, CELL_SIZE, CELL_SIZE, COLOR_SNAKE);

        ioctl(fb_fd as usize, FB_FLUSH, 0);

        while get_time() < frame_start_time + 200 {
            let mut event = InputEvent { timestamp_usec: 0, event_type: 0, code: 0, value: 0 };
            let event_slice = core::ptr::slice_from_raw_parts_mut(&mut event as *mut InputEvent as *mut u8, core::mem::size_of::<InputEvent>());
            let ret = unsafe { read(kb_fd as usize, &mut *event_slice) };
            
            // If we detect a keypress, add it to our queue (max 2)
            if ret > 0 && event.event_type == EV_KEY && event.value == 1 { 
                if input_count < 2 {
                    input_buffer[input_count] = event.code;
                    input_count += 1;
                }
            }
        }
    }

    0
}

fn fill_background(fb: *mut u8) {
    for y in 0..SCREEN_H {
        for x in 0..SCREEN_W {
            set_pixel(fb, x, y, COLOR_BG);
        }
    }
}

fn draw_play_area(fb: *mut u8) {
    let play_w = GRID_W * CELL_SIZE;
    let play_h = GRID_H * CELL_SIZE;

    for gy in 0..GRID_H {
        for gx in 0..GRID_W {
            let px = OFFSET_X + gx * CELL_SIZE;
            let py = OFFSET_Y + gy * CELL_SIZE;
            for i in 0..CELL_SIZE {
                set_pixel(fb, px + i, py, COLOR_GRID);
                set_pixel(fb, px, py + i, COLOR_GRID);
            }
        }
    }

    for x in 0..=play_w {
        set_pixel(fb, OFFSET_X + x, OFFSET_Y - 1, COLOR_BORDER);
        set_pixel(fb, OFFSET_X + x, OFFSET_Y + play_h, COLOR_BORDER);
    }
    for y in 0..=play_h {
        set_pixel(fb, OFFSET_X - 1, OFFSET_Y + y, COLOR_BORDER);
        set_pixel(fb, OFFSET_X + play_w, OFFSET_Y + y, COLOR_BORDER);
    }
}

// Fills a solid rectangle without an inset (used for clearing text)
fn fill_rect(fb: *mut u8, x: usize, y: usize, w: usize, h: usize, color: (u8, u8, u8)) {
    for dy in 0..h {
        for dx in 0..w {
            set_pixel(fb, x + dx, y + dy, color);
        }
    }
}

fn draw_rect(fb: *mut u8, x: usize, y: usize, w: usize, h: usize, color: (u8, u8, u8)) {
    let inset = 1;
    for dy in inset..(h - inset) {
        for dx in inset..(w - inset) {
            set_pixel(fb, x + dx, y + dy, color);
        }
    }
}

fn set_pixel(fb: *mut u8, x: usize, y: usize, color: (u8, u8, u8)) {
    if x >= SCREEN_W || y >= SCREEN_H { return; }
    let offset = (y * SCREEN_W + x) * 4;
    unsafe {
        fb.add(offset).write_volatile(color.2);     // B
        fb.add(offset + 1).write_volatile(color.1); // G
        fb.add(offset + 2).write_volatile(color.0); // R
        fb.add(offset + 3).write_volatile(0xff);    // A
    }
}

fn draw_number(fb: *mut u8, mut x: usize, y: usize, mut num: u32, color: (u8, u8, u8)) {
    let mut digits = [0u8; 10];
    let mut count = 0;
    
    if num == 0 {
        digits[0] = 0;
        count = 1;
    } else {
        while num > 0 {
            digits[count] = (num % 10) as u8;
            num /= 10;
            count += 1;
        }
    }

    let scale = 4;

    for i in (0..count).rev() {
        let d = digits[i] as usize;
        let bitmap = &DIGITS[d];

        for row in 0..5 {
            for col in 0..3 {
                if bitmap[row * 3 + col] == 1 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            set_pixel(fb, x + col * scale + dx, y + row * scale + dy, color);
                        }
                    }
                }
            }
        }
        x += 4 * scale;
    }
}