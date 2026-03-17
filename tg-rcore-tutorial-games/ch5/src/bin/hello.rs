#![no_std]
#![no_main]

extern crate games_lib;

use games_lib::println;

#[unsafe(no_mangle)]
fn main() -> i32 {
    println!("Hello there!");
    0
}