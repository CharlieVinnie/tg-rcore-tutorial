#![no_std]
#![no_main]

extern crate games_lib;

use games_lib::{println, fork};

#[unsafe(no_mangle)]
fn main() -> i32 {
    let child = "Hello from child!";
    let parent = "Hello from parent!";
    if fork() == 0 {
        println!("{}", child);
    } else {
        println!("{}", parent);
    }
    0
}