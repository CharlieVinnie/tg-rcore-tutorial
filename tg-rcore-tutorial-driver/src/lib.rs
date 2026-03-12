#![no_std]

extern crate tg_console;
extern crate alloc;
mod gpu;
mod input;
mod devices;
mod plic;
mod buffer;
mod qemu;

pub use devices::DeviceManager;

pub use gpu::{GpuDevice, VirtIOGpuWrapper};