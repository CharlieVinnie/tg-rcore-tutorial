/// This provides keyboard and mouse input

use core::any::Any;
use spin::Mutex;
use virtio_drivers::{Hal, MmioTransport, VirtIOInput};

use crate::{buffer::RingBuffer, clock, devices::Device};

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub timestamp: u64,
    pub event_type: u16,
    pub code: u16,
    pub value: u32,
}

struct VirtIOInputInner<H: Hal> {
    virtio_input: VirtIOInput<H, MmioTransport>,
    events: RingBuffer<InputEvent, 128>, // TODO: configurable
}

pub struct VirtIOInputWrapper<H: Hal> {
    inner: Mutex<VirtIOInputInner<H>>,
}

pub trait InputDevice: Device + Send + Sync + Any {
    fn read_event(&self) -> Option<InputEvent>;
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

    fn read_event(&self) -> Option<InputEvent> {
        self.inner.lock().events.pop()
    }
}

impl<H: Hal + 'static> Device for VirtIOInputWrapper<H> {
    fn handle_irq(&self) {
        // TODO: exclusive_session?
        let mut inner = self.inner.lock();

        inner.virtio_input.ack_interrupt();
        while let Some(event) = inner.virtio_input.pop_pending_event() {
            let result = InputEvent {
                timestamp: clock::get_time_us() as u64,
                event_type: event.event_type,
                code: event.code,
                value: event.value
            };
            let _ = inner.events.push(result);
        }
    }
}