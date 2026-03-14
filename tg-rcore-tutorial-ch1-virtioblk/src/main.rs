//! 最小化的 RISC-V S 态 VirtIO Block Driver

#![no_std]
#![no_main]
#![deny(warnings, missing_docs)]
#![allow(dead_code)]

use tg_sbi::{console_putchar, shutdown};
use core::fmt::{self, Write};

/// S 态程序入口点。
#[cfg(target_arch = "riscv64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
unsafe extern "C" fn _start() -> ! {
    const STACK_SIZE: usize = 16 * 4096;
    #[unsafe(link_section = ".boot.stack")]
    static mut STACK: [u8; STACK_SIZE] = [0xCC; STACK_SIZE];

    core::arch::naked_asm!(
        "la sp, {stack} + {stack_size}",
        "j  {main}",
        stack_size = const STACK_SIZE,
        stack      =   sym STACK,
        main       =   sym rust_main,
    )
}

struct Stdout;
impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.bytes() {
            console_putchar(c);
        }
        Ok(())
    }
}

/// 控制台打印辅助函数
pub fn print_fmt(args: fmt::Arguments) {
    Stdout.write_fmt(args).unwrap();
}

/// `print!` 宏
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::print_fmt(format_args!($($arg)*));
    };
}

/// `println!` 宏
#[macro_export]
macro_rules! println {
    ($fmt:literal $(, $($arg: tt)+)?) => {
        $crate::print!(concat!($fmt, "\n") $(, $($arg)+)?);
    }
}

fn clear_bss() {
    unsafe {
        unsafe extern "C" {
            fn sbss();
            fn ebss();
        }
        let mut ptr = sbss as *mut u8;
        let end = ebss as *mut u8;
        while ptr < end {
            ptr.write_volatile(0);
            ptr = ptr.add(1);
        }
    }
}

const VIRTIO_MAGIC: u32 = 0x74726976;
const VIRTIO_ID_BLOCK: u32 = 2;
const QUEUE_SIZE: usize = 16;

#[repr(C, align(4096))]
struct DmaBuffer([u8; 4096 * 16]);
static mut DMA_BUF: DmaBuffer = DmaBuffer([0; 4096 * 16]);
static mut DMA_OFFSET: usize = 0;

/// Abstract memory allocation
struct Dma {
    paddr: usize,
    pages: usize,
}
impl Dma {
    fn new(pages: usize) -> Self {
        let paddr = unsafe {
            let base = core::ptr::addr_of_mut!(DMA_BUF) as usize;
            let addr = base + DMA_OFFSET;
            DMA_OFFSET += pages * 4096;
            addr
        };
        // Zero out memory
        unsafe {
            core::ptr::write_bytes(paddr as *mut u8, 0, pages * 4096);
        }
        Self { paddr, pages }
    }
}

#[repr(C)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}
const VRING_DESC_F_NEXT: u16 = 1;
const VRING_DESC_F_WRITE: u16 = 2;

#[repr(C)]
struct VirtqAvail {
    flags: u16,
    idx: u16,
    ring: [u16; QUEUE_SIZE],
    used_event: u16,
}

#[repr(C)]
struct VirtqUsedElem {
    id: u32,
    len: u32,
}

#[repr(C)]
struct VirtqUsed {
    flags: u16,
    idx: u16,
    ring: [VirtqUsedElem; QUEUE_SIZE],
    avail_event: u16,
}

#[repr(C)]
struct VirtioBlkReq {
    type_: u32,
    reserved: u32,
    sector: u64,
}
const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;

// VirtIO MMIO Register Offsets
const VIRTIO_REG_MAGIC_VALUE: usize = 0x000;
const VIRTIO_REG_VERSION: usize = 0x004;
const VIRTIO_REG_DEVICE_ID: usize = 0x008;
const VIRTIO_REG_DEVICE_FEATURES: usize = 0x010;
const VIRTIO_REG_DRIVER_FEATURES: usize = 0x020;
const VIRTIO_REG_GUEST_PAGE_SIZE: usize = 0x028;
const VIRTIO_REG_QUEUE_SEL: usize = 0x030;
const VIRTIO_REG_QUEUE_NUM_MAX: usize = 0x034;
const VIRTIO_REG_QUEUE_NUM: usize = 0x038;
const VIRTIO_REG_QUEUE_ALIGN: usize = 0x03c;
const VIRTIO_REG_QUEUE_PFN: usize = 0x040;
const VIRTIO_REG_QUEUE_NOTIFY: usize = 0x050;
const VIRTIO_REG_STATUS: usize = 0x070;

// VirtIO Device Status Bits
const VIRTIO_STATUS_RESET: u32 = 0;
const VIRTIO_STATUS_ACKNOWLEDGE: u32 = 1;
const VIRTIO_STATUS_DRIVER: u32 = 2;
const VIRTIO_STATUS_DRIVER_OK: u32 = 4;
const VIRTIO_STATUS_FEATURES_OK: u32 = 8;

const PAGE_SIZE: u32 = 4096;

fn discover_virtio_block_device(fdt_paddr: usize) -> Option<usize> {
    let fdt = unsafe { fdt::Fdt::from_ptr(fdt_paddr as *const u8).unwrap() };
    for node in fdt.all_nodes() {
        if let Some(compatible) = node.compatible() {
            if compatible.all().any(|s| s == "virtio,mmio") {
                if let Some(reg) = node.reg().and_then(|mut r| r.next()) {
                    let addr = reg.starting_address as usize;
                    let magic = unsafe { ((addr + VIRTIO_REG_MAGIC_VALUE) as *const u32).read_volatile() };
                    let device_id = unsafe { ((addr + VIRTIO_REG_DEVICE_ID) as *const u32).read_volatile() };
                    
                    if magic == VIRTIO_MAGIC && device_id == VIRTIO_ID_BLOCK {
                        println!("--> Found VirtIO Block Device at {:#x}", addr);
                        return Some(addr);
                    }
                }
            }
        }
    }
    None
}

struct VirtioBlock {
    base: usize,
    desc_area: *mut VirtqDesc,
    avail_ring: *mut VirtqAvail,
    used_ring: *mut VirtqUsed,
    free_head: u16,
    avail_idx: u16,
    last_used_idx: u16,
}

impl VirtioBlock {
    fn new(base: usize) -> Self {
        Self { 
            base,
            desc_area: core::ptr::null_mut(),
            avail_ring: core::ptr::null_mut(),
            used_ring: core::ptr::null_mut(),
            free_head: 0,
            avail_idx: 0,
            last_used_idx: 0,
        }
    }

    fn read_reg(&self, offset: usize) -> u32 {
        unsafe { ((self.base + offset) as *const u32).read_volatile() }
    }

    fn write_reg(&mut self, offset: usize, val: u32) {
        unsafe { ((self.base + offset) as *mut u32).write_volatile(val) }
    }

    fn init(&mut self) {
        println!("Starting VirtIO Initialization Sequence...");
        let version = self.read_reg(VIRTIO_REG_VERSION);
        println!("VirtIO MMIO Version: {}", version);

        self.write_reg(VIRTIO_REG_STATUS, VIRTIO_STATUS_RESET); 
        self.write_reg(VIRTIO_REG_STATUS, VIRTIO_STATUS_ACKNOWLEDGE); 
        let mut status = self.read_reg(VIRTIO_REG_STATUS);
        status |= VIRTIO_STATUS_DRIVER; 
        self.write_reg(VIRTIO_REG_STATUS, status);
        
        let _features = self.read_reg(VIRTIO_REG_DEVICE_FEATURES);
        self.write_reg(VIRTIO_REG_DRIVER_FEATURES, 0); // No special features requested
        
        // Note: FEATURES_OK is VirtIO v1.0 (MMIO v2), doing it on v1 might fail or be ignored.
        // On Legacy (v1), we just skip to Queue Setup.
        
        self.write_reg(VIRTIO_REG_GUEST_PAGE_SIZE, PAGE_SIZE); // GuestPageSize (Legacy V1 only)

        // Queue Setup
        self.write_reg(VIRTIO_REG_QUEUE_SEL, 0); // QueueSel
        let max_num = self.read_reg(VIRTIO_REG_QUEUE_NUM_MAX);
        if max_num == 0 {
            panic!("Queue 0 is not available!");
        }
        self.write_reg(VIRTIO_REG_QUEUE_NUM, QUEUE_SIZE as u32); // QueueNum
        self.write_reg(VIRTIO_REG_QUEUE_ALIGN, PAGE_SIZE); // QueueAlign

        // For MMIO Version 1, the queue is a contiguous block of memory
        // Descriptor table: 16 bytes * QUEUE_SIZE = 256 bytes
        // Avail ring: 6 chars + 2 * QUEUE_SIZE = 38 bytes
        // Padding to QueueAlign boundary (4096)
        // Used ring: 6 chars + 8 * QUEUE_SIZE = 134 bytes
        // Total needed: 2 pages (8192 bytes)
        let queue_dma = Dma::new(2);
        
        self.desc_area = queue_dma.paddr as *mut VirtqDesc;
        self.avail_ring = (queue_dma.paddr + 256) as *mut VirtqAvail;
        self.used_ring = (queue_dma.paddr + PAGE_SIZE as usize) as *mut VirtqUsed;

        // Initialize free list
        for i in 0..QUEUE_SIZE - 1 {
            unsafe {
                (*self.desc_area.add(i)).next = (i + 1) as u16;
            }
        }

        // Write QueuePFN (Page Frame Number)
        self.write_reg(VIRTIO_REG_QUEUE_PFN, (queue_dma.paddr / PAGE_SIZE as usize) as u32);
        
        // Ready isn't strict in v1, but okay if device accepts it? Actually QueueReady is v2.
        // We will just not write QueueReady.

        status |= VIRTIO_STATUS_DRIVER_OK; // DRIVER_OK
        self.write_reg(VIRTIO_REG_STATUS, status);
        println!("DRIVER_OK set. Queue is ready!");
    }

    /// Abstract Enqueue Operation
    fn enqueue(&mut self, req: &VirtioBlkReq, buf: *mut u8, len: u32, is_write: bool, status: *mut u8) {
        let head = self.free_head;
        let desc1 = head;
        let desc2 = unsafe { (*self.desc_area.add(desc1 as usize)).next };
        let desc3 = unsafe { (*self.desc_area.add(desc2 as usize)).next };
        self.free_head = unsafe { (*self.desc_area.add(desc3 as usize)).next };

        unsafe {
            (*self.desc_area.add(desc1 as usize)).addr = req as *const _ as u64;
            (*self.desc_area.add(desc1 as usize)).len = core::mem::size_of::<VirtioBlkReq>() as u32;
            (*self.desc_area.add(desc1 as usize)).flags = VRING_DESC_F_NEXT;
            (*self.desc_area.add(desc1 as usize)).next = desc2;

            (*self.desc_area.add(desc2 as usize)).addr = buf as u64;
            (*self.desc_area.add(desc2 as usize)).len = len;
            (*self.desc_area.add(desc2 as usize)).flags = VRING_DESC_F_NEXT | if is_write { 0 } else { VRING_DESC_F_WRITE };
            (*self.desc_area.add(desc2 as usize)).next = desc3;

            (*self.desc_area.add(desc3 as usize)).addr = status as u64;
            (*self.desc_area.add(desc3 as usize)).len = 1;
            (*self.desc_area.add(desc3 as usize)).flags = VRING_DESC_F_WRITE;
            (*self.desc_area.add(desc3 as usize)).next = 0;

            let avail = self.avail_ring;
            (*avail).ring[(self.avail_idx as usize) % QUEUE_SIZE] = head;
            core::arch::asm!("fence w, w"); // Ensure desc written before idx
            self.avail_idx = self.avail_idx.wrapping_add(1);
            (*avail).idx = self.avail_idx;
            core::arch::asm!("fence w, w"); // Ensure idx written before notify
        }

        self.write_reg(VIRTIO_REG_QUEUE_NOTIFY, 0); // QueueNotify index 0
        println!("  [enqueue] Notified device.");
    }

    /// Abstract Dequeue Operation (Wait for used)
    fn wait_for_used(&mut self) -> u16 {
        println!("  [wait_for_used] Waiting for device... last_used_idx = {}", self.last_used_idx);
        unsafe {
            let used = self.used_ring;
            loop {
                let idx = core::ptr::read_volatile(&(*used).idx);
                if idx != self.last_used_idx {
                    println!("  [wait_for_used] Device responded! new idx = {}", idx);
                    break;
                }
                core::arch::asm!("nop");
            }
            core::arch::asm!("fence r, rw");
            
            let used_elem = &(*used).ring[(self.last_used_idx as usize) % QUEUE_SIZE];
            let head = used_elem.id as u16;
            
            // Reclaim
            let mut curr = head;
            while ((*self.desc_area.add(curr as usize)).flags & VRING_DESC_F_NEXT) != 0 {
                curr = (*self.desc_area.add(curr as usize)).next;
            }
            (*self.desc_area.add(curr as usize)).next = self.free_head;
            self.free_head = head;

            self.last_used_idx = self.last_used_idx.wrapping_add(1);
            head
        }
    }

    // High level abstractions Phase 6 cleanly separated into functions
    fn write_sector(&mut self, sector: u64, buf: &[u8]) {
        assert!(buf.len() <= 512); // Sector size
        let mut padded_buf = [0u8; 512];
        padded_buf[..buf.len()].copy_from_slice(buf);

        let req = VirtioBlkReq {
            type_: VIRTIO_BLK_T_OUT,
            reserved: 0,
            sector,
        };
        let mut status: u8 = 0xFF;

        self.enqueue(&req, padded_buf.as_ptr() as *mut u8, 512, true, &mut status as *mut u8);
        self.wait_for_used();

        let s = unsafe { core::ptr::read_volatile(&status) };
        if s != 0 {
            panic!("Write failed with status {}", s);
        }
        println!("Successfully wrote {} bytes to sector {}", buf.len(), sector);
    }

    fn read_sector(&mut self, sector: u64, buf: &mut [u8]) {
        assert!(buf.len() >= 512);

        let req = VirtioBlkReq {
            type_: VIRTIO_BLK_T_IN,
            reserved: 0,
            sector,
        };
        let mut status: u8 = 0xFF;

        self.enqueue(&req, buf.as_mut_ptr(), 512, false, &mut status as *mut u8);
        self.wait_for_used();

        let s = unsafe { core::ptr::read_volatile(&status) };
        if s != 0 {
            panic!("Read failed with status {}", s);
        }
        println!("Successfully read sector {}", sector);
    }
}

/// S 态主函数
#[unsafe(no_mangle)]
extern "C" fn rust_main(hartid: usize, fdt_paddr: usize) -> ! {
    clear_bss();
    println!("============================================");
    println!("Hello to Bare-metal VirtIO Block Driver!");
    println!("============================================");
    println!("Hart ID: {}", hartid);

    let mmio_base = discover_virtio_block_device(fdt_paddr).expect("VirtIO block device not found!");
    
    let mut blk = VirtioBlock::new(mmio_base);
    blk.init();

    // The "Hello World" Execution Flow
    let message = b"Hello World!";
    println!("Part A: The Write Operation");
    blk.write_sector(0, message);

    println!("Part B: The Read Operation");
    let mut read_buf = [0u8; 512];
    blk.read_sector(0, &mut read_buf);
    
    // Print the string from read_buf
    let mut end_idx = 0;
    while end_idx < read_buf.len() && read_buf[end_idx] != 0 {
        end_idx += 1;
    }
    
    let result_str = core::str::from_utf8(&read_buf[..end_idx]).unwrap();
    println!("Read back from disk: '{}'", result_str);

    println!("Demo completed successfully!");
    shutdown(false);
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("KERNEL PANIC: {}", info);
    shutdown(true)
}
