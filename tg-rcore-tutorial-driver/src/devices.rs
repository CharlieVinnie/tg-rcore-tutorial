use alloc::{collections::btree_map::BTreeMap, sync::Arc};
use tg_console::println;
use virtio_drivers::{DeviceType, Hal, MmioTransport, Transport, VirtIOHeader};

use crate::{GpuDevice, VirtIOGpuWrapper, buffer::OverflowStrategy, input::{InputDevice, VirtIOInputWrapper}, qemu::{claim_irq, complete_irq, setup_interrupt_for, enable_external_interrupts}};

pub const VIRTIO_START: usize = 0x1000_1000;
pub const VIRTIO_END: usize = 0x1000_9000;
pub const VIRTIO_STEP: usize = 0x1000;

pub trait Device : Send + Sync {
    fn handle_irq(&self);
}

pub struct VirtIOProbingIterator {
    current: usize
}

impl VirtIOProbingIterator {
    pub fn new() -> Self {
        Self {
            current: 0
        }
    }
}

impl Iterator for VirtIOProbingIterator {
    type Item = (usize, &'static mut VirtIOHeader);

    fn next(&mut self) -> Option<Self::Item> {
        let addr = VIRTIO_START + self.current * VIRTIO_STEP;
        if addr >= VIRTIO_END {
            return None;
        } else {
            self.current += 1;
            // Note that addr 9x1000_1000 has id 1, so add self.current first
            Some((self.current, unsafe { &mut *(addr as *mut VirtIOHeader) }))
        }
    }
}

pub struct DeviceManager {
    gpu: Option<Arc<dyn GpuDevice>>,
    keyboard: Option<Arc<dyn InputDevice>>,
    irq_map: BTreeMap<usize, Arc<dyn Device>>,
}

impl DeviceManager {
    pub fn new<H: Hal + 'static>() -> Self {
        let mut manager = Self {
            gpu: None,
            keyboard: None,
            irq_map: BTreeMap::new(),
        };

        let mut devices = VirtIOProbingIterator::new();

        while let Some((slot, header)) = devices.next() {
            let Ok(transport) = (unsafe { MmioTransport::new(core::ptr::NonNull::from(header)) }) else {
                continue;
            };
            match transport.device_type() {
                DeviceType::GPU if manager.gpu.is_none() => {
                    if let Ok(gpu) = VirtIOGpuWrapper::<H>::new(transport) {
                        let gpu = Arc::new(gpu);
                        manager.gpu = Some(gpu.clone());
                        manager.irq_map.insert(slot, gpu.clone() as _);
                        // Interrupts for GPU aren't quite needed now
                        // setup_interrupt_for(slot);
                        println!("GPU device found at slot {slot}");
                    }
                }
                DeviceType::Input if manager.keyboard.is_none() => {
                    if let Ok(keyboard) = VirtIOInputWrapper::<H>::new(transport, OverflowStrategy::DropOldest) {
                        let keyboard = Arc::new(keyboard);
                        manager.keyboard = Some(keyboard.clone());
                        manager.irq_map.insert(slot, keyboard.clone() as _);
                        setup_interrupt_for(slot);
                        println!("Keyboard device found at slot {slot}");
                    }
                }
                _ => {}
            }
        }

        enable_external_interrupts();

        manager
    }

    fn get_device(&self, slot: u32) -> Option<Arc<dyn Device>> {
        self.irq_map.get(&(slot as usize)).cloned()
    }

    pub fn get_gpu(&self) -> Option<Arc<dyn GpuDevice>> {
        self.gpu.clone()
    }

    pub fn get_keyboard(&self) -> Option<Arc<dyn InputDevice>> {
        self.keyboard.clone()
    }

    pub fn handle_external_interrupt(&self) {
        let Some(slot) = claim_irq() else { return; };
        self.get_device(slot).unwrap().handle_irq();
        complete_irq(slot);
    }
}