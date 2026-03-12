#![no_std]
#![no_main]

extern crate games_lib;

use games_lib::{open, mmap, ioctl, OpenFlags};

const FB_FLUSH: usize = 1;
const SHAPE_ID: usize = 5;

#[unsafe(no_mangle)]
fn main() -> i32 {
    let fd = open("/dev/fb0\0", OpenFlags::RDWR);
    if fd < 0 { return -1; }

    let fb_base = mmap(0, 800 * 600 * 4, 0, 0, fd as usize, 0);
    if fb_base == 0 { return -1; }

    let fb_ptr = fb_base as *mut u8;

    if SHAPE_ID == 0 {
        fill_background(fb_ptr);
    }
    draw_shape(fb_ptr, SHAPE_ID);

    ioctl(fd as usize, FB_FLUSH, 0);
    0
}

const TANGRAM_POLYGONS: &[(&[(i32, i32)], (u8, u8, u8))] = &[
    // --- Shape "0" (Left) ---
    (&[(152, 69), (54, 69), (54, 165)], (200, 20, 10)),                // Red Tri
    (&[(153, 70), (54, 166), (54, 358), (153, 263)], (255, 210, 10)),   // Yellow Para
    (&[(153, 69), (352, 69), (352, 262)], (240, 80, 220)),             // Pink Tri
    (&[(252, 166), (252, 358), (352, 455), (352, 263)], (60, 30, 240)), // Blue Para
    (&[(54, 359), (54, 552), (251, 552)], (10, 200, 245)),             // Cyan Tri
    (&[(252, 359), (153, 456), (253, 552), (352, 455)], (100, 240, 40)), // Green Sq

    // --- Shape "5" (Right) ---
    (&[(600, 69), (500, 166), (598, 262)], (10, 200, 245)),            // Cyan Tri
    (&[(600, 69), (698, 69), (698, 165)], (60, 30, 240)),              // Blue Tri
    (&[(797, 50), (698, 50), (698, 166), (797, 108)], (240, 80, 220)), // Pink Poly
    (&[(599, 262), (599, 358), (698, 358), (698, 262)], (100, 240, 40)), // Green Sq
    (&[(699, 263), (700, 455), (797, 358)], (240, 80, 220)),           // Pink Tri
    (&[(500, 455), (550, 552), (648, 552), (599, 455)], (255, 140, 0)), // Orange Para
    (&[(599, 455), (649, 552), (699, 455)], (60, 30, 240)),            // Blue Tri
];

/// Ray-casting algorithm for point-in-polygon testing
fn is_inside(x: i32, y: i32, points: &[(i32, i32)]) -> bool {
    let mut inside = false;
    let mut j = points.len() - 1;
    for i in 0..points.len() {
        let (pi_x, pi_y) = points[i];
        let (pj_x, pj_y) = points[j];

        if ((pi_y > y) != (pj_y > y)) &&
           (x < (pj_x - pi_x) * (y - pi_y) / (pj_y - pi_y + 1) + pi_x) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn fill_background(fb: *mut u8) {
    let width = 800;

    // Fill background (Dark grey 0x333333)
    for y in 0..600 {
        for x in 0..800 {
            let offset = (y * width + x) * 4;
            unsafe {
                fb.add(offset).write_volatile(0x33);     // B
                fb.add(offset + 1).write_volatile(0x33); // G
                fb.add(offset + 2).write_volatile(0x33); // R
                fb.add(offset + 3).write_volatile(0xff); // A
            }
        }
    }
} 

fn draw_shape(fb: *mut u8, shape_id: usize) {
    let width = 800;

    let &(points, color) = &TANGRAM_POLYGONS[shape_id];

    // Find bounding box
    let mut min_x = 800; let mut max_x = 0;
    let mut min_y = 600; let mut max_y = 0;

    for &(x, y) in points {
        if x < min_x { min_x = x; }
        if x > max_x { max_x = x; }
        if y < min_y { min_y = y; }
        if y > max_y { max_y = y; }
    }

    // Clamp to screen bounds
    min_x = min_x.max(0); max_x = max_x.min(799);
    min_y = min_y.max(0); max_y = max_y.min(599);

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            if is_inside(x, y, points) {
                let offset = (y as usize * width as usize + x as usize) * 4;
                unsafe {
                    fb.add(offset).write_volatile(color.2);     // B
                    fb.add(offset + 1).write_volatile(color.1); // G
                    fb.add(offset + 2).write_volatile(color.0); // R
                    fb.add(offset + 3).write_volatile(0xff);    // A
                }
            }
        }
    }
}
