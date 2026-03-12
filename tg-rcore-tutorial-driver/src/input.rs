/// This provides keyboard and mouse input

use core::any::Any;
use spin::Mutex;
use virtio_drivers::{Hal, MmioTransport, VirtIOInput};

use crate::{buffer::RingBuffer, devices::Device};

struct VirtIOInputInner<H: Hal> {
    virtio_input: VirtIOInput<H, MmioTransport>,
    events: RingBuffer<u64, 128>, // TODO: configurable
}

pub struct VirtIOInputWrapper<H: Hal> {
    inner: Mutex<VirtIOInputInner<H>>,
}

pub trait InputDevice: Device + Send + Sync + Any {
    fn read_event(&self) -> Option<u64>;
    fn is_empty(&self) -> bool;
}

impl<H: Hal> VirtIOInputWrapper<H> {
    pub fn new(transport: MmioTransport, buffer_overflow_strategy: crate::buffer::OverflowStrategy) -> Result<Self, virtio_drivers::Error> {
        let inner = VirtIOInputInner {
            virtio_input: VirtIOInput::new(transport)?,
            events: RingBuffer::new(buffer_overflow_strategy),
        };
        Ok(Self {
            inner: Mutex::new(inner),
        })
    }
}

unsafe impl<H: Hal> Send for VirtIOInputWrapper<H> {}
unsafe impl<H: Hal> Sync for VirtIOInputWrapper<H> {}

impl<H: Hal + 'static> InputDevice for VirtIOInputWrapper<H> {
    fn is_empty(&self) -> bool {
        self.inner.lock().events.is_empty()
    }

    fn read_event(&self) -> Option<u64> {
        self.inner.lock().events.pop()
    }
}

impl<H: Hal + 'static> Device for VirtIOInputWrapper<H> {
    fn handle_irq(&self) {
        let mut result;
        // TODO: exclusive_session?
        let mut inner = self.inner.lock();

        inner.virtio_input.ack_interrupt();
        while let Some(event) = inner.virtio_input.pop_pending_event() {
            result = (event.event_type as u64) << 48
                | (event.code as u64) << 32
                | (event.value) as u64;
            let _ = inner.events.push(result);
        }
    }
}