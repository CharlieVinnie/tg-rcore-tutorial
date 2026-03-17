#![no_std]
#![no_main]
#![allow(clippy::println_empty_string)]

extern crate alloc;

#[macro_use]
extern crate games_lib;

const LF: u8 = 0x0au8;
const CR: u8 = 0x0du8;
const DL: u8 = 0x7fu8;
const BS: u8 = 0x08u8;
const ESC: u8 = 0x1bu8; // Added Escape character for arrow keys

use alloc::string::String;
use alloc::vec::Vec;
use games_lib::{exec, fork, getchar, waitpid};

// 教学目标：
// 实现一个带有历史记录、输入过滤和基本控制序列解析的用户态 shell。

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    println!("Rust user shell");
    let mut line: String = String::new(); // 当前输入的命令
    let mut history: Vec<String> = Vec::new(); // 历史命令记录
    let mut history_idx: usize = 0; // 当前浏览的历史命令索引

    print!(">> ");
    loop {
        let c = getchar();
        match c {
            LF | CR => {
                // 换行
                println!();
                if !line.is_empty() {
                    // Save to history and reset the history index
                    history.push(line.clone());
                    history_idx = history.len();

                    let pid = fork();
                    if pid == 0 {
                        // child process
                        if exec(line.as_str()) == -1 {
                            println!("Error when executing!");
                            return -4;
                        }
                        unreachable!();
                    } else {
                        // 父进程等待子进程完成，打印退出信息。
                        let mut exit_code: i32 = 0;
                        let exit_pid = waitpid(pid as isize, &mut exit_code);
                        assert_eq!(pid, exit_pid);
                        println!("Shell: Process {} exited with code {}", pid, exit_code);
                    }
                    line.clear();
                }
                print!(">> ");
            }
            BS | DL => {
                // backspace
                // The check `!line.is_empty()` is what mathematically guarantees 
                // you cannot backspace out your prompt (">> ").
                if !line.is_empty() {
                    print!("{} {}", BS as char, BS as char); // BS, space, BS clears the char on screen
                    line.pop();
                }
            }
            ESC => {
                // 捕捉 ANSI 转义序列 (Escape Sequences) 比如方向键
                let c2 = getchar();
                if c2 == b'[' {
                    let c3 = getchar();
                    match c3 {
                        b'A' => {
                            // 上箭头 (Up Arrow)
                            if history_idx > 0 {
                                history_idx -= 1;
                                
                                // 清除屏幕上的当前行
                                for _ in 0..line.len() {
                                    print!("{} {}", BS as char, BS as char);
                                }
                                
                                line.clear();
                                line.push_str(&history[history_idx]);
                                print!("{}", line);
                            }
                        }
                        b'B' => {
                            // 下箭头 (Down Arrow)
                            if history_idx < history.len() {
                                history_idx += 1;
                                
                                // 清除屏幕上的当前行
                                for _ in 0..line.len() {
                                    print!("{} {}", BS as char, BS as char);
                                }
                                
                                line.clear();
                                if history_idx < history.len() {
                                    line.push_str(&history[history_idx]);
                                }
                                print!("{}", line);
                            }
                        }
                        _ => {} // 忽略其他转义序列
                    }
                }
            }
            _ => {
                // 限制：仅允许可打印的 ASCII 字符 (范围 0x20 空格 到 0x7E 波浪号)
                if (0x20..=0x7E).contains(&c) {
                    print!("{}", c as char);
                    line.push(c as char);
                }
            }
        }
    }
}