// /// This provides keyboard and mouse input

// use core::any::Any;
// use spin::Mutex;
// use virtio_drivers::{MmioTransport, VirtIOHeader, VirtIOInput};

// struct VirtIOInputInner {
//     virtio_input: VirtIOInput<VirtioHal, MmioTransport>,
//     events: VecDeque<u64>,
// }

// struct VirtIOInputWrapper {
//     inner: Mutex<VirtIOInputInner>,
//     condvar: Condvar,
// }

// pub trait InputDevice: Send + Sync + Any {
//     fn read_event(&self) -> u64;
//     fn handle_irq(&self);
//     fn is_empty(&self) -> bool;
// }

// impl VirtIOInputWrapper {
//     pub fn new(header: &'static mut VirtIOHeader) -> Result<Self, virtio_drivers::Error> {
//         let transport = unsafe { virtio_drivers::MmioTransport::new(core::ptr::NonNull::from(header)) }
//             .map_err(|_| virtio_drivers::Error::InvalidParam)?;

//         let inner = VirtIOInputInner {
//             virtio_input: unsafe {
//                 VirtIOInput::<VirtioHal>::new(&mut *(addr as *mut VirtIOHeader)).unwrap()
//             },
//             events: VecDeque::new(),
//         };
//         Self {
//             inner: unsafe { UPIntrFreeCell::new(inner) },
//             condvar: Condvar::new(),
//         }
//     }
// }

// impl InputDevice for VirtIOInputWrapper {
//     fn is_empty(&self) -> bool {
//         self.inner.exclusive_access().events.is_empty()
//     }

//     fn read_event(&self) -> u64 {
//         loop {
//             let mut inner = self.inner.exclusive_access();
//             if let Some(event) = inner.events.pop_front() {
//                 return event;
//             } else {
//                 let task_cx_ptr = self.condvar.wait_no_sched();
//                 drop(inner);
//                 schedule(task_cx_ptr);
//             }
//         }
//     }

//     fn handle_irq(&self) {
//         let mut count = 0;
//         let mut result = 0;
//         self.inner.exclusive_session(|inner| {
//             inner.virtio_input.ack_interrupt();
//             while let Some(event) = inner.virtio_input.pop_pending_event() {
//                 count += 1;
//                 result = (event.event_type as u64) << 48
//                     | (event.code as u64) << 32
//                     | (event.value) as u64;
//                 inner.events.push_back(result);
//             }
//         });
//         if count > 0 {
//             self.condvar.signal();
//         };
//     }
// }
