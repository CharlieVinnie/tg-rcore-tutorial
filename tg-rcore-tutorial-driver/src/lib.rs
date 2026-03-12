#![no_std]

extern crate alloc;
mod gpu;
mod input;
mod devices;
mod plic;

pub use devices::DeviceManager;

pub use gpu::{GpuDevice, VirtIOGpuWrapper};