Now please implement the game of ch2 in tg-rcore-tutorial-games in src/bin/ch2b_os[1-13].rs

```
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
```

Each program program should draw exactly one of these parts. You can choose to put this array in all programs, but each program has its unique id and prints the shape of that id.

Then, modify tg-rcore-tutorial-ch2/build.rs so that it uses ensure_tg_games just like ch1, but looks for cases.toml in tutorial-games/ instead. You might want to modify the [ch2] part of that .toml. Others shall be modified later.

Transfer the syscalls implemented in ch1 to ch2.