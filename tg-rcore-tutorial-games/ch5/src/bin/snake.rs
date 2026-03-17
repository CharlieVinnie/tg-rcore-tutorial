#![no_std]
#![no_main]

use games_lib::{OpenFlags, get_time, ioctl, mmap, open, println, read};
use games_lib::constants::*;

extern crate games_lib;

// Grid configurations (Screen config removed for dynamic resolution)
const CELL_SIZE: usize = 20;
const GRID_W: usize = 20;
const GRID_H: usize = 20;

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

// Helper to spawn food avoiding the snake's body
fn spawn_food(prng: &mut Prng, snake: &[(i32, i32)], snake_len: usize) -> (i32, i32) {
    loop {
        let fx = prng.next_range(0, GRID_W as u32) as i32;
        let fy = prng.next_range(0, GRID_H as u32) as i32;
        
        let mut on_snake = false;
        for i in 0..snake_len {
            if snake[i] == (fx, fy) {
                on_snake = true;
                break;
            }
        }
        
        if !on_snake {
            return (fx, fy);
        }
    }
}

// --- Framebuffer Wrapper ---
// This bundles all the dynamic resolution data so we don't have to pass dimensions everywhere
struct Display {
    ptr: *mut u8,
    w: usize,
    h: usize,
    offset_x: usize,
    offset_y: usize,
}

impl Display {
    fn set_pixel(&self, x: usize, y: usize, color: (u8, u8, u8)) {
        if x >= self.w || y >= self.h { return; }
        let offset = (y * self.w + x) * 4;
        unsafe {
            self.ptr.add(offset).write_volatile(color.2);     // B
            self.ptr.add(offset + 1).write_volatile(color.1); // G
            self.ptr.add(offset + 2).write_volatile(color.0); // R
            self.ptr.add(offset + 3).write_volatile(0xff);    // A
        }
    }

    fn fill_background(&self) {
        for y in 0..self.h {
            for x in 0..self.w {
                self.set_pixel(x, y, COLOR_BG);
            }
        }
    }

    fn draw_play_area(&self) {
        let play_w = GRID_W * CELL_SIZE;
        let play_h = GRID_H * CELL_SIZE;

        for gy in 0..GRID_H {
            for gx in 0..GRID_W {
                let px = self.offset_x + gx * CELL_SIZE;
                let py = self.offset_y + gy * CELL_SIZE;
                for i in 0..CELL_SIZE {
                    self.set_pixel(px + i, py, COLOR_GRID);
                    self.set_pixel(px, py + i, COLOR_GRID);
                }
            }
        }

        for x in 0..=play_w {
            self.set_pixel(self.offset_x + x, self.offset_y - 1, COLOR_BORDER);
            self.set_pixel(self.offset_x + x, self.offset_y + play_h, COLOR_BORDER);
        }
        for y in 0..=play_h {
            self.set_pixel(self.offset_x - 1, self.offset_y + y, COLOR_BORDER);
            self.set_pixel(self.offset_x + play_w, self.offset_y + y, COLOR_BORDER);
        }
    }

    fn fill_rect(&self, x: usize, y: usize, w: usize, h: usize, color: (u8, u8, u8)) {
        for dy in 0..h {
            for dx in 0..w {
                self.set_pixel(x + dx, y + dy, color);
            }
        }
    }

    fn draw_rect(&self, x: usize, y: usize, w: usize, h: usize, color: (u8, u8, u8)) {
        let inset = 1;
        for dy in inset..(h - inset) {
            for dx in inset..(w - inset) {
                self.set_pixel(x + dx, y + dy, color);
            }
        }
    }

    fn draw_number(&self, mut x: usize, y: usize, mut num: u32, color: (u8, u8, u8)) {
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
                                self.set_pixel(x + col * scale + dx, y + row * scale + dy, color);
                            }
                        }
                    }
                }
            }
            x += 4 * scale;
        }
    }
}

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

    // --- Dynamic Resolution Setup ---
    let mut res: (u32, u32) = (0, 0);
    ioctl(fb_fd as usize, FB_GET_RESOLUTION, &mut res as *mut _ as usize);
    let screen_w = res.0 as usize;
    let screen_h = res.1 as usize;

    let fb_base = mmap(0, screen_w * screen_h * 4, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, fb_fd as usize, 0);
    if fb_base < 0 { return -1; }

    println!("fb_base: {:#x}", fb_base);

    // Dynamically calculate offsets to perfectly center the game board
    let display = Display {
        ptr: fb_base as *mut u8,
        w: screen_w,
        h: screen_h,
        offset_x: (screen_w.saturating_sub(GRID_W * CELL_SIZE)) / 2,
        offset_y: (screen_h.saturating_sub(GRID_H * CELL_SIZE)) / 2,
    };

    let kb_fd = open("/dev/input0\0", OpenFlags::RDWR);
    if kb_fd < 0 { return -1; }

    // Game State
    let mut snake = [(0i32, 0i32); MAX_SNAKE_LEN];
    let mut snake_len = 4;
    
    for i in 0..snake_len {
        snake[i] = ((GRID_W / 2) as i32 - i as i32, (GRID_H / 2) as i32);
    }

    let mut dir = (1i32, 0i32);
    let mut prng = Prng { state: 1234567 };
    
    // Use the safe spawn method
    let mut food = spawn_food(&mut prng, &snake, snake_len);
    let mut score: u32 = 0;

    // --- INITIAL RENDER ---
    display.fill_background();
    display.draw_play_area();
    
    for i in 0..snake_len {
        display.draw_rect(display.offset_x + snake[i].0 as usize * CELL_SIZE, display.offset_y + snake[i].1 as usize * CELL_SIZE, CELL_SIZE, CELL_SIZE, COLOR_SNAKE);
    }
    display.draw_rect(display.offset_x + food.0 as usize * CELL_SIZE, display.offset_y + food.1 as usize * CELL_SIZE, CELL_SIZE, CELL_SIZE, COLOR_FOOD);
    
    let score_y = display.offset_y.saturating_sub(40);
    display.draw_number(display.offset_x, score_y, score, COLOR_BORDER);
    
    ioctl(fb_fd as usize, FB_FLUSH, 0);

    let mut input_buffer: [u16; 2] = [0, 0];
    let mut input_count = 0;

    loop {
        let frame_start_time = get_time();

        // --- 2. Process Buffered Input ---
        if input_count > 0 {
            let code = input_buffer[0];
            
            input_buffer[0] = input_buffer[1];
            input_count -= 1;

            match code {
                KEY_UP | KEY_W    if dir.1 == 0 => dir = (0, -1),
                KEY_DOWN | KEY_S  if dir.1 == 0 => dir = (0, 1),
                KEY_LEFT | KEY_A  if dir.0 == 0 => dir = (-1, 0),
                KEY_RIGHT | KEY_D if dir.0 == 0 => dir = (1, 0),
                _ => {} 
            }
        }

        // --- 3. Update State ---
        let head = snake[0];
        let new_head = (head.0 + dir.0, head.1 + dir.1);

        if new_head.0 < 0 || new_head.0 >= GRID_W as i32 || new_head.1 < 0 || new_head.1 >= GRID_H as i32 {
            break; 
        }

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
            
            // Generate food guaranteeing it avoids the snake body
            food = spawn_food(&mut prng, &snake, snake_len);
            
            display.draw_rect(display.offset_x + food.0 as usize * CELL_SIZE, display.offset_y + food.1 as usize * CELL_SIZE, CELL_SIZE, CELL_SIZE, COLOR_FOOD);
            
            display.fill_rect(display.offset_x, score_y, 120, 30, COLOR_BG);
            display.draw_number(display.offset_x, score_y, score, COLOR_BORDER);
        }

        for i in (1..snake_len).rev() {
            snake[i] = snake[i - 1];
        }
        snake[0] = new_head;

        // --- 4. Render Delta ---
        if !ate_food {
            display.draw_rect(display.offset_x + old_tail.0 as usize * CELL_SIZE, display.offset_y + old_tail.1 as usize * CELL_SIZE, CELL_SIZE, CELL_SIZE, COLOR_BG);
        }

        display.draw_rect(display.offset_x + new_head.0 as usize * CELL_SIZE, display.offset_y + new_head.1 as usize * CELL_SIZE, CELL_SIZE, CELL_SIZE, COLOR_SNAKE);

        ioctl(fb_fd as usize, FB_FLUSH, 0);

        while get_time() < frame_start_time + 200 {
            let mut event = InputEvent { timestamp_usec: 0, event_type: 0, code: 0, value: 0 };
            let event_slice = core::ptr::slice_from_raw_parts_mut(&mut event as *mut InputEvent as *mut u8, core::mem::size_of::<InputEvent>());
            let ret = unsafe { read(kb_fd as usize, &mut *event_slice) };
            
            if ret > 0 && event.event_type == EV_KEY && event.value == 1 { 
                if input_count < 2 {
                    input_buffer[input_count] = event.code;
                    input_count += 1;
                }
            }
        }
    }

    println!("Game Over!");

    0
}