#![no_std]

extern crate tg_console;
extern crate alloc;
mod gpu;
mod input;
mod devices;
mod plic;
mod buffer;
mod qemu;
mod clock;
mod address;

pub use devices::{DeviceManager};

pub use address::*;

pub use gpu::{GpuDevice, VirtIOGpuWrapper};

pub use input::{InputEvent};

pub use virtio_drivers::Hal;