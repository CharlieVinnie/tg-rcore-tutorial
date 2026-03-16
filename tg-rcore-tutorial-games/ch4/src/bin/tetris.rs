#![no_std]
#![no_main]

use games_lib::{OpenFlags, get_time, ioctl, mmap, open, read};

extern crate games_lib;

const FB_FLUSH: usize = 1;
const FB_GET_RESOLUTION: usize = 2;

const CELL_SIZE: usize = 25;
const BOARD_W: usize = 10;
const BOARD_H: usize = 20;

// Input Events
const EV_KEY: u16 = 1;
const KEY_Q: u16 = 16;
const KEY_W: u16 = 17;
const KEY_A: u16 = 30;
const KEY_S: u16 = 31;
const KEY_D: u16 = 32;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_Z: u16 = 44;
const KEY_C: u16 = 46;
const KEY_SPACE: u16 = 57;
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

// Colors
const COLOR_BG: (u8, u8, u8) = (35, 30, 30);
const COLOR_GRID: (u8, u8, u8) = (60, 50, 50);
const COLOR_BORDER: (u8, u8, u8) = (255, 255, 255);

const COLORS: [(u8, u8, u8); 8] = [
    (35, 30, 30),     // 0: Empty/BG
    (0, 255, 255),    // 1: I (Cyan)
    (0, 0, 255),      // 2: J (Blue)
    (255, 165, 0),    // 3: L (Orange)
    (255, 255, 0),    // 4: O (Yellow)
    (0, 255, 0),      // 5: S (Green)
    (128, 0, 128),    // 6: T (Purple)
    (255, 0, 0),      // 7: Z (Red)
];

// Tetromino Shapes (Fixed I-piece for perfect SRS rotation)
const SHAPES: [[u16; 4]; 8] = [
    [0, 0, 0, 0], // 0: Empty
    [0x0F00, 0x2222, 0x00F0, 0x4444], // 1: I (Proper SRS states)
    [0x8E00, 0x6440, 0x0E20, 0x44C0], // 2: J
    [0x2E00, 0x4460, 0x0E80, 0xC440], // 3: L
    [0x6600, 0x6600, 0x6600, 0x6600], // 4: O
    [0x6C00, 0x4620, 0x06C0, 0x8C40], // 5: S
    [0x4E00, 0x4640, 0x0E40, 0x4C40], // 6: T
    [0xC600, 0x2640, 0x0C60, 0x4C80], // 7: Z
];

// SRS Kick Tables (Adjusted for +Y = Down screen coordinates)
// Transitions: 0: 0->1, 1: 1->0, 2: 1->2, 3: 2->1, 4: 2->3, 5: 3->2, 6: 3->0, 7: 0->3
const KICKS_JLSTZ: [[(i32, i32); 5]; 8] = [
    [(0, 0), (-1, 0), (-1, -1), (0,  2), (-1,  2)], // 0->1 (CW)
    [(0, 0), ( 1, 0), ( 1,  1), (0, -2), ( 1, -2)], // 1->0 (CCW)
    [(0, 0), ( 1, 0), ( 1,  1), (0, -2), ( 1, -2)], // 1->2 (CW)
    [(0, 0), (-1, 0), (-1, -1), (0,  2), (-1,  2)], // 2->1 (CCW)
    [(0, 0), ( 1, 0), ( 1, -1), (0,  2), ( 1,  2)], // 2->3 (CW)
    [(0, 0), (-1, 0), (-1,  1), (0, -2), (-1, -2)], // 3->2 (CCW)
    [(0, 0), (-1, 0), (-1,  1), (0, -2), (-1, -2)], // 3->0 (CW)
    [(0, 0), ( 1, 0), ( 1, -1), (0,  2), ( 1,  2)], // 0->3 (CCW)
];

const KICKS_I: [[(i32, i32); 5]; 8] = [
    [(0, 0), (-2, 0), ( 1, 0), (-2,  1), ( 1, -2)], // 0->1
    [(0, 0), ( 2, 0), (-1, 0), ( 2, -1), (-1,  2)], // 1->0
    [(0, 0), (-1, 0), ( 2, 0), (-1, -2), ( 2,  1)], // 1->2
    [(0, 0), ( 1, 0), (-2, 0), ( 1,  2), (-2, -1)], // 2->1
    [(0, 0), ( 2, 0), (-1, 0), ( 2, -1), (-1,  2)], // 2->3
    [(0, 0), (-2, 0), ( 1, 0), (-2,  1), ( 1, -2)], // 3->2
    [(0, 0), ( 1, 0), (-2, 0), ( 1,  2), (-2, -1)], // 3->0
    [(0, 0), (-1, 0), ( 2, 0), (-1, -2), ( 2,  1)], // 0->3
];

fn piece_at(t: u8, rot: u8, x: i32, y: i32) -> bool {
    if x < 0 || x > 3 || y < 0 || y > 3 { return false; }
    let shape = SHAPES[t as usize][rot as usize];
    let bit_idx = (3 - y) * 4 + (3 - x);
    (shape & (1 << bit_idx)) != 0
}

fn get_kick_index(rot: u8, new_rot: u8) -> usize {
    match (rot, new_rot) {
        (0, 1) => 0, (1, 0) => 1, (1, 2) => 2, (2, 1) => 3,
        (2, 3) => 4, (3, 2) => 5, (3, 0) => 6, (0, 3) => 7,
        _ => 0,
    }
}

struct Prng { state: u32 }
impl Prng {
    fn next(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(1664525).wrapping_add(1013904223);
        self.state
    }
}

struct Randomizer {
    prng: Prng,
    bag: [u8; 7],
    index: usize,
}

impl Randomizer {
    fn new(seed: u32) -> Self {
        Self { prng: Prng { state: seed }, bag: [1, 2, 3, 4, 5, 6, 7], index: 7 }
    }
    
    fn shuffle(&mut self) {
        for i in 0..6 {
            let j = i + (self.prng.next() % (7 - i as u32)) as usize;
            let tmp = self.bag[i];
            self.bag[i] = self.bag[j];
            self.bag[j] = tmp;
        }
    }
    
    fn next_piece(&mut self) -> u8 {
        if self.index >= 7 {
            self.shuffle();
            self.index = 0;
        }
        let p = self.bag[self.index];
        self.index += 1;
        p
    }

    fn peek(&self) -> u8 {
        if self.index >= 7 { 1 } else { self.bag[self.index] }
    }
}

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
            self.ptr.add(offset).write_volatile(color.2);
            self.ptr.add(offset + 1).write_volatile(color.1);
            self.ptr.add(offset + 2).write_volatile(color.0);
            self.ptr.add(offset + 3).write_volatile(0xff);
        }
    }

    fn fill_rect(&self, x: usize, y: usize, w: usize, h: usize, color: (u8, u8, u8)) {
        for dy in 0..h {
            for dx in 0..w { self.set_pixel(x + dx, y + dy, color); }
        }
    }

    fn draw_rect(&self, x: usize, y: usize, w: usize, h: usize, color: (u8, u8, u8), inset: usize) {
        for dy in inset..(h - inset) {
            for dx in inset..(w - inset) { self.set_pixel(x + dx, y + dy, color); }
        }
    }

    fn draw_char(&self, x: usize, y: usize, c: u8, color: (u8, u8, u8), scale: usize) {
        let bitmap = if c >= b'A' && c <= b'Z' { &ALPHABET[(c - b'A') as usize] }
                     else if c >= b'0' && c <= b'9' { &DIGITS[(c - b'0') as usize] }
                     else if c == b'/' { &SLASH }
                     else { return; };

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
    }

    fn draw_string(&self, x: usize, y: usize, s: &str, color: (u8, u8, u8), scale: usize) {
        let mut cx = x;
        for b in s.bytes() {
            if b == b' ' { cx += 4 * scale; continue; }
            self.draw_char(cx, y, b, color, scale);
            cx += 4 * scale;
        }
    }

    fn draw_number(&self, x: usize, y: usize, mut num: u32, color: (u8, u8, u8), scale: usize) {
        let mut digits = [0u8; 10];
        let mut count = 0;
        
        if num == 0 { digits[0] = 0; count = 1; } 
        else {
            while num > 0 {
                digits[count] = (num % 10) as u8;
                num /= 10;
                count += 1;
            }
        }

        let mut cx = x;
        for i in (0..count).rev() {
            self.draw_char(cx, y, digits[i] + b'0', color, scale);
            cx += 4 * scale;
        }
    }
}

const SLASH: [u8; 15] = [0,0,1, 0,0,1, 0,1,0, 1,0,0, 1,0,0];
const DIGITS: [[u8; 15]; 10] = [
    [1,1,1, 1,0,1, 1,0,1, 1,0,1, 1,1,1], [0,1,0, 1,1,0, 0,1,0, 0,1,0, 1,1,1],
    [1,1,1, 0,0,1, 1,1,1, 1,0,0, 1,1,1], [1,1,1, 0,0,1, 1,1,1, 0,0,1, 1,1,1],
    [1,0,1, 1,0,1, 1,1,1, 0,0,1, 0,0,1], [1,1,1, 1,0,0, 1,1,1, 0,0,1, 1,1,1],
    [1,1,1, 1,0,0, 1,1,1, 1,0,1, 1,1,1], [1,1,1, 0,0,1, 0,0,1, 0,0,1, 0,0,1],
    [1,1,1, 1,0,1, 1,1,1, 1,0,1, 1,1,1], [1,1,1, 1,0,1, 1,1,1, 0,0,1, 1,1,1],
];
const ALPHABET: [[u8; 15]; 26] = [
    [1,1,1, 1,0,1, 1,1,1, 1,0,1, 1,0,1], [1,1,0, 1,0,1, 1,1,0, 1,0,1, 1,1,0],
    [1,1,1, 1,0,0, 1,0,0, 1,0,0, 1,1,1], [1,1,0, 1,0,1, 1,0,1, 1,0,1, 1,1,0],
    [1,1,1, 1,0,0, 1,1,1, 1,0,0, 1,1,1], [1,1,1, 1,0,0, 1,1,1, 1,0,0, 1,0,0],
    [1,1,1, 1,0,0, 1,0,1, 1,0,1, 1,1,1], [1,0,1, 1,0,1, 1,1,1, 1,0,1, 1,0,1],
    [1,1,1, 0,1,0, 0,1,0, 0,1,0, 1,1,1], [0,0,1, 0,0,1, 0,0,1, 1,0,1, 1,1,1],
    [1,0,1, 1,1,0, 1,1,0, 1,0,1, 1,0,1], [1,0,0, 1,0,0, 1,0,0, 1,0,0, 1,1,1],
    [1,0,1, 1,1,1, 1,0,1, 1,0,1, 1,0,1], [1,1,1, 1,0,1, 1,0,1, 1,0,1, 1,0,1],
    [1,1,1, 1,0,1, 1,0,1, 1,0,1, 1,1,1], [1,1,1, 1,0,1, 1,1,1, 1,0,0, 1,0,0],
    [1,1,1, 1,0,1, 1,0,1, 1,1,1, 0,0,1], [1,1,1, 1,0,1, 1,1,0, 1,0,1, 1,0,1],
    [1,1,1, 1,0,0, 1,1,1, 0,0,1, 1,1,1], [1,1,1, 0,1,0, 0,1,0, 0,1,0, 0,1,0],
    [1,0,1, 1,0,1, 1,0,1, 1,0,1, 1,1,1], [1,0,1, 1,0,1, 1,0,1, 1,0,1, 0,1,0],
    [1,0,1, 1,0,1, 1,0,1, 1,1,1, 1,0,1], [1,0,1, 1,0,1, 0,1,0, 1,0,1, 1,0,1],
    [1,0,1, 1,0,1, 0,1,0, 0,1,0, 0,1,0], [1,1,1, 0,0,1, 0,1,0, 1,0,0, 1,1,1],
];

fn check_collision(board: &[[u8; BOARD_W]; BOARD_H], t: u8, rot: u8, cx: i32, cy: i32) -> bool {
    for y in 0..4 {
        for x in 0..4 {
            if piece_at(t, rot, x, y) {
                let bx = cx + x;
                let by = cy + y;
                if bx < 0 || bx >= BOARD_W as i32 || by >= BOARD_H as i32 { return true; }
                if by >= 0 && board[by as usize][bx as usize] != 0 { return true; }
            }
        }
    }
    false
}

fn try_rotate(board: &[[u8; BOARD_W]; BOARD_H], t: u8, rot: &mut u8, cx: &mut i32, cy: &mut i32, cw: bool) -> bool {
    let new_rot = if cw { (*rot + 1) % 4 } else { (*rot + 3) % 4 };

    // The O-piece does not kick in standard SRS
    if t == 4 {
        if !check_collision(board, t, new_rot, *cx, *cy) {
            *rot = new_rot;
            return true;
        }
        return false;
    }

    let kick_idx = get_kick_index(*rot, new_rot);
    let kicks = if t == 1 { KICKS_I[kick_idx] } else { KICKS_JLSTZ[kick_idx] };
    
    for &(dx, dy) in &kicks {
        if !check_collision(board, t, new_rot, *cx + dx, *cy + dy) {
            *rot = new_rot;
            *cx += dx;
            *cy += dy;
            return true;
        }
    }
    false
}

#[unsafe(no_mangle)]
fn main() -> i32 {
    let fb_fd = open("/dev/fb0\0", OpenFlags::RDWR);
    if fb_fd < 0 { return -1; }

    let mut res: (u32, u32) = (0, 0);
    ioctl(fb_fd as usize, FB_GET_RESOLUTION, &mut res as *mut _ as usize);
    let screen_w = res.0 as usize;
    let screen_h = res.1 as usize;

    let fb_base = mmap(0, screen_w * screen_h * 4, 7, 0, fb_fd as usize, 0);
    if fb_base == 0 { return -1; }

    let play_w = BOARD_W * CELL_SIZE;
    let play_h = BOARD_H * CELL_SIZE;

    let display = Display {
        ptr: fb_base as *mut u8,
        w: screen_w,
        h: screen_h,
        offset_x: screen_w.saturating_sub(play_w) / 2,
        offset_y: screen_h.saturating_sub(play_h) / 2,
    };

    let kb_fd = open("/dev/input0\0", OpenFlags::RDWR);
    if kb_fd < 0 { return -1; }

    // Game State
    let mut board = [[0u8; BOARD_W]; BOARD_H];
    let mut bag = Randomizer::new(882931);
    
    let mut current_t = bag.next_piece();
    let mut current_rot = 0;
    let mut cx = 3;
    let mut cy = 0;

    let mut hold_t: u8 = 0;
    let mut can_hold = true;

    let mut score = 0;
    let mut lines = 0;
    let mut level = 1;
    let mut game_over = false;

    // Static UI Initialization
    display.fill_rect(0, 0, screen_w, screen_h, COLOR_BG);
    
    let info_x = display.offset_x + play_w + 30;
    let hold_x = display.offset_x.saturating_sub(130);
    let info_y = display.offset_y;

    display.draw_string(hold_x, info_y, "HOLD", COLOR_BORDER, 2);
    display.draw_string(info_x, info_y, "NEXT", COLOR_BORDER, 2);
    display.draw_string(info_x, info_y + 100, "SCORE", COLOR_BORDER, 2);
    display.draw_string(info_x, info_y + 150, "LEVEL", COLOR_BORDER, 2);
    display.draw_string(info_x, info_y + 200, "LINES", COLOR_BORDER, 2);

    display.draw_string(info_x, info_y + 270, "CONTROLS", COLOR_BORDER, 2);
    display.draw_string(info_x, info_y + 300, "UP/W ROTATE", COLOR_BORDER, 1);
    display.draw_string(info_x, info_y + 320, "Z    CCW ROT", COLOR_BORDER, 1);
    display.draw_string(info_x, info_y + 340, "A/D  MOVE L/R", COLOR_BORDER, 1);
    display.draw_string(info_x, info_y + 360, "S    DROP", COLOR_BORDER, 1);
    display.draw_string(info_x, info_y + 380, "SPC  HARD", COLOR_BORDER, 1);
    display.draw_string(info_x, info_y + 400, "C/SH HOLD", COLOR_BORDER, 1);
    display.draw_string(info_x, info_y + 420, "Q    QUIT", COLOR_BORDER, 1);

    let mut last_drop = get_time();
    let mut force_redraw = true;

    loop {
        let now = get_time();
        let drop_delay = (1000u64.saturating_sub((level as u64 - 1) * 80)).max(100) as _;

        // Input Processing
        while get_time() < now + 5 {
            let mut event = InputEvent { timestamp_usec: 0, event_type: 0, code: 0, value: 0 };
            let event_slice = core::ptr::slice_from_raw_parts_mut(&mut event as *mut _ as *mut u8, core::mem::size_of::<InputEvent>());
            let ret = unsafe { read(kb_fd as usize, &mut *event_slice) };
            
            if ret > 0 && event.event_type == EV_KEY && (event.value == 1 || event.value == 2) {
                let mut moved = false;
                match event.code {
                    KEY_LEFT | KEY_A => {
                        if !check_collision(&board, current_t, current_rot, cx - 1, cy) { cx -= 1; moved = true; }
                    }
                    KEY_RIGHT | KEY_D => {
                        if !check_collision(&board, current_t, current_rot, cx + 1, cy) { cx += 1; moved = true; }
                    }
                    KEY_DOWN | KEY_S => {
                        if !check_collision(&board, current_t, current_rot, cx, cy + 1) { 
                            cy += 1; 
                            last_drop = get_time();
                            moved = true; 
                            score += 1; 
                        }
                    }
                    KEY_UP | KEY_W => { moved = try_rotate(&board, current_t, &mut current_rot, &mut cx, &mut cy, true); }
                    KEY_Z => { moved = try_rotate(&board, current_t, &mut current_rot, &mut cx, &mut cy, false); }
                    KEY_SPACE => {
                        let mut drop_y = cy;
                        while !check_collision(&board, current_t, current_rot, cx, drop_y + 1) { drop_y += 1; }
                        score += (drop_y - cy) as u32 * 2;
                        cy = drop_y;
                        last_drop = 0;
                        moved = true;
                    }
                    KEY_C | KEY_LEFTSHIFT => {
                        if can_hold {
                            if hold_t == 0 {
                                hold_t = current_t;
                                current_t = bag.next_piece();
                            } else {
                                let tmp = current_t;
                                current_t = hold_t;
                                hold_t = tmp;
                            }
                            current_rot = 0;
                            cx = 3;
                            cy = 0;
                            last_drop = get_time();
                            can_hold = false;
                            moved = true;
                        }
                    }
                    KEY_Q => { return 0; }
                    _ => {}
                }
                if moved { force_redraw = true; }
            }
        }

        // Gravity / Lock Processing
        if get_time() - last_drop > drop_delay {
            if !check_collision(&board, current_t, current_rot, cx, cy + 1) {
                cy += 1;
                last_drop = get_time();
                force_redraw = true;
            } else if !game_over {
                // Lock Piece
                for y in 0..4 {
                    for x in 0..4 {
                        if piece_at(current_t, current_rot, x, y) {
                            let by = (cy + y) as usize;
                            let bx = (cx + x) as usize;
                            if by < BOARD_H && bx < BOARD_W {
                                board[by][bx] = current_t;
                            }
                        }
                    }
                }

                // Line Clears
                let mut cleared = 0;
                let mut check_y = BOARD_H as i32 - 1;
                while check_y >= 0 {
                    let mut full = true;
                    for x in 0..BOARD_W {
                        if board[check_y as usize][x] == 0 { full = false; break; }
                    }
                    if full {
                        cleared += 1;
                        for sy in (1..=check_y as usize).rev() {
                            board[sy] = board[sy - 1];
                        }
                        board[0] = [0; BOARD_W];
                    } else {
                        check_y -= 1;
                    }
                }

                if cleared > 0 {
                    lines += cleared;
                    level = 1 + lines / 10;
                    score += match cleared { 1 => 100, 2 => 300, 3 => 500, _ => 800 } * level;
                }

                // Spawn Next Piece
                current_t = bag.next_piece();
                current_rot = 0;
                cx = 3; 
                cy = 0;
                last_drop = get_time();
                can_hold = true;
                force_redraw = true;

                if check_collision(&board, current_t, current_rot, cx, cy) {
                    game_over = true;
                }
            }
        }

        // Rendering Delta
        if force_redraw {
            display.draw_rect(display.offset_x - 1, display.offset_y - 1, play_w + 2, play_h + 2, COLOR_BORDER, 0);

            for y in 0..BOARD_H {
                for x in 0..BOARD_W {
                    let t = board[y][x];
                    let px = display.offset_x + x * CELL_SIZE;
                    let py = display.offset_y + y * CELL_SIZE;
                    let color = if t == 0 { COLOR_BG } else { COLORS[t as usize] };
                    let inset = if t == 0 { 0 } else { 1 };
                    
                    display.fill_rect(px, py, CELL_SIZE, CELL_SIZE, COLOR_BG);
                    display.draw_rect(px, py, CELL_SIZE, CELL_SIZE, color, inset);
                    if t == 0 { display.set_pixel(px + CELL_SIZE/2, py + CELL_SIZE/2, COLOR_GRID); }
                }
            }

            if !game_over {
                let mut ghost_y = cy;
                while !check_collision(&board, current_t, current_rot, cx, ghost_y + 1) { ghost_y += 1; }

                for y in 0..4 {
                    for x in 0..4 {
                        if piece_at(current_t, current_rot, x, y) {
                            let block_color = COLORS[current_t as usize];
                            
                            if ghost_y >= 0 {
                                let g_px = display.offset_x + (cx + x) as usize * CELL_SIZE;
                                let g_py = display.offset_y + (ghost_y + y) as usize * CELL_SIZE;
                                display.draw_rect(g_px, g_py, CELL_SIZE, CELL_SIZE, (block_color.0/3, block_color.1/3, block_color.2/3), 2);
                            }
                            
                            if cy + y >= 0 {
                                let a_px = display.offset_x + (cx + x) as usize * CELL_SIZE;
                                let a_py = display.offset_y + (cy + y) as usize * CELL_SIZE;
                                display.draw_rect(a_px, a_py, CELL_SIZE, CELL_SIZE, block_color, 1);
                            }
                        }
                    }
                }
            }

            // Update UI Numbers
            display.fill_rect(info_x, info_y + 120, 100, 20, COLOR_BG);
            display.draw_number(info_x, info_y + 120, score, COLOR_BORDER, 2);
            
            display.fill_rect(info_x, info_y + 170, 100, 20, COLOR_BG);
            display.draw_number(info_x, info_y + 170, level, COLOR_BORDER, 2);
            
            display.fill_rect(info_x, info_y + 220, 100, 20, COLOR_BG);
            display.draw_number(info_x, info_y + 220, lines, COLOR_BORDER, 2);

            // Draw Next Box Content
            display.fill_rect(info_x, info_y + 30, CELL_SIZE * 4, CELL_SIZE * 4, COLOR_BG);
            let next_t = bag.peek();
            for y in 0..4 {
                for x in 0..4 {
                    if piece_at(next_t, 0, x, y) {
                        display.draw_rect(info_x + x as usize * CELL_SIZE, info_y + 30 + y as usize * CELL_SIZE, CELL_SIZE, CELL_SIZE, COLORS[next_t as usize], 1);
                    }
                }
            }

            // Draw Hold Box Content
            display.fill_rect(hold_x, info_y + 30, CELL_SIZE * 4, CELL_SIZE * 4, COLOR_BG);
            if hold_t != 0 {
                let h_color = if can_hold { COLORS[hold_t as usize] } else { (100, 100, 100) };
                for y in 0..4 {
                    for x in 0..4 {
                        if piece_at(hold_t, 0, x, y) {
                            display.draw_rect(hold_x + x as usize * CELL_SIZE, info_y + 30 + y as usize * CELL_SIZE, CELL_SIZE, CELL_SIZE, h_color, 1);
                        }
                    }
                }
            }

            if game_over {
                display.draw_string(display.offset_x + 30, display.offset_y + play_h / 2, "GAME OVER", COLOR_BORDER, 3);
            }

            ioctl(fb_fd as usize, FB_FLUSH, 0);
            force_redraw = false;
        }
    }
}