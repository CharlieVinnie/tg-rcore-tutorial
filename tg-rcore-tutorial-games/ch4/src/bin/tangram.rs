#![no_std]
#![no_main]

extern crate games_lib;

use games_lib::{OpenFlags, ioctl, mmap, open, println};

const FB_FLUSH: usize = 1;
const FB_GET_RESOLUTION: usize = 2;

#[unsafe(no_mangle)]
fn main() -> i32 {
    let fd = open("/dev/fb0\0", OpenFlags::RDWR);
    if fd < 0 { return -1; }

    let mut resolution: (u32, u32) = (0, 0);
    ioctl(fd as _, FB_GET_RESOLUTION, &mut resolution as *mut _ as usize);
    println!("resolution: {} * {}", resolution.0, resolution.1);

    // Memory map the framebuffer dynamically based on the fetched resolution
    let fb = mmap(0, (resolution.0 * resolution.1 * 4) as _, 7, 0, fd as _, 0);

    if fb < 0 { return -1; }
    
    // Pass the actual dimensions to the drawing function
    draw_tangram(fb as *mut u8, resolution.0, resolution.1);
    
    ioctl(fd as _, FB_FLUSH, 0);
    0
}

const TANGRAM_POLYGONS: &[(&[(i32, i32)], (u8, u8, u8))] = &[
    // --- Shape "0" (Left) ---
    (&[(152, 69), (54, 69), (54, 165)], (200, 20, 10)),                // Red Tri
    (&[(153, 70), (54, 166), (54, 358), (153, 263)], (255, 210, 10)),  // Yellow Para
    (&[(153, 69), (352, 69), (352, 262)], (240, 80, 220)),             // Pink Tri
    (&[(252, 166), (252, 358), (352, 455), (352, 263)], (60, 30, 240)),// Blue Para
    (&[(54, 359), (54, 552), (251, 552)], (10, 200, 245)),             // Cyan Tri
    (&[(252, 359), (153, 456), (253, 552), (352, 455)], (100, 240, 40)),// Green Sq

    // --- Shape "5" (Right) ---
    (&[(600, 69), (500, 166), (598, 262)], (10, 200, 245)),            // Cyan Tri
    (&[(600, 69), (698, 69), (698, 165)], (60, 30, 240)),              // Blue Tri
    (&[(797, 50), (698, 50), (698, 166), (797, 108)], (240, 80, 220)), // Pink Poly
    (&[(599, 262), (599, 358), (698, 358), (698, 262)], (100, 240, 40)),// Green Sq
    (&[(699, 263), (700, 455), (797, 358)], (240, 80, 220)),           // Pink Tri
    (&[(500, 455), (550, 552), (648, 552), (599, 455)], (255, 140, 0)),// Orange Para
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

fn draw_tangram(fb: *mut u8, width: u32, height: u32) {
    let w = width as i32;
    let h = height as i32;

    // 1. Calculate aspect ratio preserving dimensions (Letterboxing)
    let mapped_w: i32;
    let mapped_h: i32;
    
    // Check if the screen is wider than the original 4:3 ratio
    if w * 600 > h * 800 { 
        mapped_h = h;
        mapped_w = (h * 800) / 600;
    } else { 
        mapped_w = w;
        mapped_h = (w * 600) / 800;
    }

    let offset_x = (w - mapped_w) / 2;
    let offset_y = (h - mapped_h) / 2;

    // 2. Fill background (Dark grey 0x333333) dynamically
    for y in 0..h {
        for x in 0..w {
            let offset = (y as usize * width as usize + x as usize) * 4;
            unsafe {
                fb.add(offset).write_volatile(0x33);     // B
                fb.add(offset + 1).write_volatile(0x33); // G
                fb.add(offset + 2).write_volatile(0x33); // R
                fb.add(offset + 3).write_volatile(0xff); // A
            }
        }
    }

    // 3. Draw shapes with dynamic scaling
    for &(points, color) in TANGRAM_POLYGONS {
        // Find bounding box in the original 800x600 space
        let mut min_x = 800; let mut max_x = 0;
        let mut min_y = 600; let mut max_y = 0;

        for &(px, py) in points {
            if px < min_x { min_x = px; }
            if px > max_x { max_x = px; }
            if py < min_y { min_y = py; }
            if py > max_y { max_y = py; }
        }

        // Map the bounding box to the target screen space
        let mut screen_min_x = offset_x + (min_x * mapped_w) / 800;
        let mut screen_max_x = offset_x + (max_x * mapped_w) / 800;
        let mut screen_min_y = offset_y + (min_y * mapped_h) / 600;
        let mut screen_max_y = offset_y + (max_y * mapped_h) / 600;

        // Clamp & pad margins to prevent edge cutoff from integer rounding
        screen_min_x = (screen_min_x - 1).max(0);
        screen_max_x = (screen_max_x + 1).min(w - 1);
        screen_min_y = (screen_min_y - 1).max(0);
        screen_max_y = (screen_max_y + 1).min(h - 1);

        for y in screen_min_y..=screen_max_y {
            for x in screen_min_x..=screen_max_x {
                // Ensure we only draw inside the letterboxed area
                if x >= offset_x && x < offset_x + mapped_w && 
                   y >= offset_y && y < offset_y + mapped_h {
                    
                    // Inverse map screen (x, y) back to the 800x600 coordinate system
                    let orig_x = ((x - offset_x) * 800) / mapped_w;
                    let orig_y = ((y - offset_y) * 600) / mapped_h;

                    // Use the mapped coordinates for our ray-casting test
                    if is_inside(orig_x, orig_y, points) {
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
    }
}