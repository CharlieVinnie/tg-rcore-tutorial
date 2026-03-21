use alloc::sync::Arc;
use tg_driver::{GpuDevice, InputDevice, InputEvent};
use tg_easy_fs::{FileHandle, UserBuffer};

use crate::memory::VmMapper;

// Generic file traits
pub trait File {
    fn read(&self, buf: UserBuffer) -> isize;
    fn write(&self, buf: UserBuffer) -> isize;
    fn mmap(&self, mapper: &mut dyn VmMapper) -> Result<(), ()>;
    fn ioctl(&self, _request: usize, _argp: usize) -> isize {
        tg_console::log::error!("ioctl not supported for this file type");
        -1
    }
    fn lseek(&self, _offset: isize, _whence: usize) -> isize {
        -1
    }
}

// Files that reside on the disk
pub struct DiskFile {
    file_handle: Arc<FileHandle>,
}

// Screen as a file
pub struct GPUFile {
    fb_addr: usize,
    gpu: Arc<dyn GpuDevice>,
}

// input devices as a file
pub struct InputDevFile {
    device: Arc<dyn InputDevice>,
}

impl DiskFile {
    pub fn new(file_handle: Arc<FileHandle>) -> Self {
        Self { file_handle }
    }
}

impl GPUFile {
    pub fn new(gpu: Arc<dyn GpuDevice>) -> Self {
        Self { 
            fb_addr: gpu.get_framebuffer().unwrap().as_ptr() as _,
            gpu 
        }
    }
}

impl InputDevFile {
    pub fn new(device: Arc<dyn InputDevice>) -> Self {
        Self { device }
    }
}

impl File for DiskFile {
    fn read(&self, buf: UserBuffer) -> isize {
        self.file_handle.read(buf)
    }
    fn write(&self, buf: UserBuffer) -> isize {
        self.file_handle.write(buf)
    }
    fn mmap(&self, _mapper: &mut dyn VmMapper) -> Result<(), ()> {
        unimplemented!("mmap not implemented for DiskFile")
    }
    fn lseek(&self, offset: isize, whence: usize) -> isize {
        const SEEK_SET: usize = 0;
        const SEEK_CUR: usize = 1;
        // SEEK_END not yet supported (would need file size from inode)
        let new_offset = match whence {
            SEEK_SET => offset as usize,
            SEEK_CUR => {
                let cur = self.file_handle.offset.get();
                if offset < 0 {
                    cur.wrapping_add(offset as usize)
                } else {
                    cur + offset as usize
                }
            }
            _ => return -1,
        };
        self.file_handle.offset.set(new_offset);
        new_offset as isize
    }
}

impl File for GPUFile {
    fn read(&self, _buf: UserBuffer) -> isize {
        unimplemented!("Use mmap instead of read for GPUFile")
    }
    fn write(&self, _buf: UserBuffer) -> isize {
        unimplemented!("Use mmap instead of write for GPUFile")
    }
    fn mmap(&self, mapper: &mut dyn VmMapper) -> Result<(), ()> {
        mapper.map_to(self.fb_addr)
    }
    
    fn ioctl(&self, request: usize, argp: usize) -> isize {
        crate::device::gpu_ioctl(&self.gpu, request, argp)
    }
}

impl File for InputDevFile {
    fn read(&self, buf: UserBuffer) -> isize {
        const SIZE: usize = core::mem::size_of::<InputEvent>();
        let total_len = buf.len();
        let mut remaining_len = total_len;
        let mut iter = buf.into_iter();
        while remaining_len >= SIZE {
            if let Some(event) = self.device.read_event() {
                let raw_event = unsafe { core::slice::from_raw_parts(&event as *const InputEvent as *const u8, SIZE) };
                for byte in raw_event {
                    unsafe { iter.next().unwrap().write(*byte); }
                }
                remaining_len -= SIZE;
            } else {
                break;
            }
        }
        (total_len - remaining_len) as _
    }
    fn write(&self, _buf: UserBuffer) -> isize {
        unimplemented!("write not implemented for InputDevFile")
    }
    fn mmap(&self, _mapper: &mut dyn VmMapper) -> Result<(), ()> {
        unimplemented!("mmap not implemented for InputDevFile")
    }
}