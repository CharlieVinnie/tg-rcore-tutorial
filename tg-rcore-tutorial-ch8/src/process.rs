//! 进程管理模块
//!
//! 与第五章相比，本章的 `Process` 新增了**文件描述符表**（`fd_table`）字段，
//! 每个进程拥有自己的 fd_table，统一管理标准 I/O 和磁盘文件。
//!
//! ## 文件描述符表
//!
//! | fd | 用途 |
//! |----|------|
//! | 0 | 标准输入（stdin） |
//! | 1 | 标准输出（stdout） |
//! | 2 | 标准错误（stderr） |
//! | 3+ | 普通文件（通过 open 系统调用分配） |
//!
//! 教程阅读建议：
//!
//! - 先看 `from_elf`：理解用户地址空间与初始 fd_table 如何构建；
//! - 再看 `fork`：观察地址空间和文件描述符的继承规则；
//! - 最后看 `change_program_brk`：理解用户堆扩缩时的页映射变化。

use crate::{Sv39, Sv39Manager, build_flags, file::{DiskFile, File}, map_portal, parse_flags, user_allocator::UserAllocator};
use alloc::{alloc::alloc_zeroed, sync::Arc, vec::Vec};
use core::alloc::Layout;
use spin::Mutex;
use tg_easy_fs::{FileHandle};
use tg_kernel_context::{foreign::ForeignContext, LocalContext};
use tg_kernel_vm::{
    AddressSpace, MapVisibility, page_table::{MmuMeta, PPN, VAddr, VPN}
};
use tg_task_manage::ProcId;
use xmas_elf::{
    header::{self, HeaderPt2, Machine},
    program, ElfFile,
};

/// 进程结构体
///
/// 与第五章相比新增了 `fd_table` 字段。
pub struct Process {
    /// 进程标识符（PID），创建后不可变
    pub pid: ProcId,
    /// 用户态上下文（含 satp，支持跨地址空间切换）
    pub context: ForeignContext,
    /// 进程的独立地址空间
    pub address_space: AddressSpace<Sv39, Sv39Manager>,
    /// 文件描述符表
    ///
    /// 每个 fd 对应一个 `Option<Mutex<File>>`：
    /// - `Some(...)`: 有效的文件句柄
    /// - `None`: 该 fd 已关闭或未使用
    ///
    /// 预留 fd 0/1/2 分别为 stdin/stdout/stderr。
    pub fd_table: Vec<Option<Mutex<Arc<dyn File>>>>,
    /// 堆底地址
    pub heap_bottom: usize,
    /// 当前程序 break 位置（堆顶）
    pub program_brk: usize,
    /// for mmaps
    pub allocator: UserAllocator,
    /// Stride value for scheduling
    pub stride: usize,
    /// Priority value for scheduling
    pub priority: usize,
}

impl Process {
    /// exec：用新程序替换当前进程（保留 PID 和 fd_table）
    pub fn exec(&mut self, elf: ElfFile) {
        let proc = Process::from_elf(elf).unwrap();
        self.address_space = proc.address_space;
        self.context = proc.context;
        self.heap_bottom = proc.heap_bottom;
        self.program_brk = proc.program_brk;
    }

    /// fork：复制当前进程创建子进程
    ///
    /// 深拷贝地址空间和文件描述符表。
    /// 子进程继承父进程的所有已打开文件。
    pub fn fork(&mut self) -> Option<Process> {
        let pid = ProcId::new();
        // 复制父进程的完整地址空间
        let parent_addr_space = &self.address_space;
        let mut address_space: AddressSpace<Sv39, Sv39Manager> = AddressSpace::new();
        parent_addr_space.cloneself(&mut address_space);
        map_portal(&address_space);
        // 复制父进程上下文
        let context = self.context.context.clone();
        let satp = (8 << 60) | address_space.root_ppn().val();
        let foreign_ctx = ForeignContext { context, satp };
        // 复制父进程的文件描述符表
        // 子进程继承父进程所有已打开的文件
        let mut new_fd_table: Vec<Option<Mutex<Arc<dyn File>>>> = Vec::new();
        for fd in self.fd_table.iter_mut() {
            if let Some(file) = fd {
                new_fd_table.push(Some(Mutex::new(file.get_mut().clone())));
            } else {
                new_fd_table.push(None);
            }
        }
        Some(Self {
            pid,
            context: foreign_ctx,
            address_space,
            fd_table: new_fd_table,
            heap_bottom: self.heap_bottom,
            program_brk: self.program_brk,
            allocator: self.allocator.clone(),
            stride: 0,
            priority: self.priority,
        })
    }

    /// 从 ELF 文件创建新进程
    ///
    /// 与第五章相同的 ELF 解析流程，但新增了文件描述符表的初始化：
    /// - fd 0 = stdin（可读）
    /// - fd 1 = stdout（可写）
    /// - fd 2 = stderr（可写）
    pub fn from_elf(elf: ElfFile) -> Option<Self> {
        let entry = match elf.header.pt2 {
            HeaderPt2::Header64(pt2)
                if pt2.type_.as_type() == header::Type::Executable
                    && pt2.machine.as_machine() == Machine::RISC_V =>
            {
                pt2.entry_point as usize
            }
            _ => None?,
        };

        const PAGE_SIZE: usize = 1 << Sv39::PAGE_BITS;
        const PAGE_MASK: usize = PAGE_SIZE - 1;

        let mut address_space = AddressSpace::new();
        let mut max_end_va: usize = 0;

        let mut load_segments: alloc::vec::Vec<_> = elf
            .program_iter()
            .filter(|p| matches!(p.get_type(), Ok(program::Type::Load)))
            .collect();
        load_segments.sort_by_key(|p| p.virtual_addr());

        // 用于处理跨段共享页的缓存结构
        struct PendingPage<F> {
            vpn: VPN<Sv39>,
            flags: F,
            data: alloc::vec::Vec<u8>,
        }
        let mut pending_page: Option<PendingPage<_>> = None;

        // 遍历 ELF LOAD 段，解决重叠后映射到地址空间
        for program in &load_segments {
            let off_file = program.offset() as usize;
            let len_file = program.file_size() as usize;
            let off_mem = program.virtual_addr() as usize;
            let end_mem = off_mem + program.mem_size() as usize;
            assert_eq!(off_file & PAGE_MASK, off_mem & PAGE_MASK);

            if end_mem > max_end_va {
                max_end_va = end_mem;
            }

            let mut flags: [u8; 5] = *b"U___V";
            if program.flags().is_execute() { flags[1] = b'X'; }
            if program.flags().is_write() { flags[2] = b'W'; }
            if program.flags().is_read() { flags[3] = b'R'; }
            let seg_flags = parse_flags(unsafe { core::str::from_utf8_unchecked(&flags) }).unwrap();

            let file_data = &elf.input[off_file..off_file + len_file];

            let start_vpn = VAddr::new(off_mem).floor();
            let end_vpn = VAddr::new(end_mem).ceil();

            let mut current_vpn = start_vpn;
            let mut current_file_offset = 0;

            // 1. 检查当前段的第一页是否与上一个段的最后一页重叠
            if let Some(mut p) = pending_page.take() {
                if p.vpn == current_vpn {
                    // 发生页重叠！权限取并集
                    p.flags |= seg_flags; 

                    // 将当前段位于共享页的数据拷贝进去
                    let page_offset = off_mem & PAGE_MASK;
                    let copy_len = core::cmp::min(PAGE_SIZE - page_offset, file_data.len());
                    if copy_len > 0 {
                        p.data[page_offset..page_offset + copy_len]
                            .copy_from_slice(&file_data[..copy_len]);
                    }

                    if end_vpn == current_vpn + 1 {
                        // 当前段完全结束在这个共享页内，继续挂起
                        pending_page = Some(p);
                        continue; 
                    } else {
                        // 当前段延伸到了后面的页，重叠页已满，可以安全映射了！
                        address_space.map(
                            p.vpn..p.vpn+1,
                            &p.data,
                            0,
                            p.flags,
                            MapVisibility::PRIVATE,
                        );
                        current_vpn += 1;
                        current_file_offset += copy_len;
                    }
                } else {
                    // 没有重叠，把上一个挂起的页直接映射出去
                    address_space.map(
                        p.vpn..p.vpn+1,
                        &p.data,
                        0,
                        p.flags,
                        MapVisibility::PRIVATE,
                    );
                }
            }

            // 安全检查：如果段在重叠处理中已经被完全吃掉，跳过
            if current_vpn >= end_vpn {
                continue;
            }

            let last_vpn = VPN::new(end_vpn.val().wrapping_sub(1));

            // 2. 批量映射当前段内部完整的页面 (跳过最后一页)
            if current_vpn < last_vpn {
                let bulk_off_mem = core::cmp::max(off_mem, current_vpn.base().val());
                let bulk_end_mem = core::cmp::min(off_mem + len_file, last_vpn.base().val());
                let bulk_copy_len = bulk_end_mem.saturating_sub(bulk_off_mem);

                let bulk_data = &file_data[current_file_offset..current_file_offset + bulk_copy_len];
                let bulk_page_offset = bulk_off_mem - (current_vpn.base().val());

                address_space.map(
                    current_vpn..last_vpn,
                    bulk_data,
                    bulk_page_offset,
                    seg_flags,
                    MapVisibility::PRIVATE,
                );

                current_file_offset += bulk_copy_len;
            }

            // 3. 提取当前段的最后一页，作为新的挂起页 (Pending Page) 记录下来
            let mut p_data = alloc::vec![0u8; PAGE_SIZE];
            let last_page_va = last_vpn.base().val();
            let last_off_mem = core::cmp::max(off_mem, last_page_va);
            let last_page_offset = last_off_mem - last_page_va;

            let remaining_file_data = file_data.len().saturating_sub(current_file_offset);
            let copy_len = core::cmp::min(remaining_file_data, PAGE_SIZE - last_page_offset);

            if copy_len > 0 {
                p_data[last_page_offset..last_page_offset + copy_len]
                    .copy_from_slice(&file_data[current_file_offset..current_file_offset + copy_len]);
            }

            pending_page = Some(PendingPage {
                vpn: last_vpn,
                flags: seg_flags,
                data: p_data,
            });
        }

        // 4. 所有段遍历结束，刷出最后剩下的挂起页
        if let Some(p) = pending_page.take() {
            address_space.map(
                p.vpn..p.vpn+1,
                &p.data,
                0,
                p.flags,
                MapVisibility::PRIVATE,
            );
        }

        // 堆底从 ELF 加载的最高地址的下一页开始
        let heap_bottom = VAddr::<Sv39>::new(max_end_va).ceil().base().val();

        // 映射用户栈（2 页 = 8 KiB）
        let stack = unsafe {
            alloc_zeroed(Layout::from_size_align_unchecked(
                2 << Sv39::PAGE_BITS,
                1 << Sv39::PAGE_BITS,
            ))
        };
        address_space.map_extern(
            VPN::new((1 << 26) - 2)..VPN::new(1 << 26),
            PPN::new(stack as usize >> Sv39::PAGE_BITS),
            build_flags("U_WRV"),
            MapVisibility::PRIVATE,
        );
        // 映射异界传送门
        map_portal(&address_space);

        // 创建用户态上下文
        let mut context = LocalContext::user(entry);
        let satp = (8 << 60) | address_space.root_ppn().val();
        *context.sp_mut() = 1 << 38;

        // area reserved for mmap
        let mmap_start = VPN::<Sv39>::new((1 << 26) - 2000).base().val();
        let mmap_end = VPN::<Sv39>::new((1 << 26) - 5).base().val();
        let allocator = UserAllocator::new(
            mmap_start..mmap_end
        );

        Some(Self {
            pid: ProcId::new(),
            context: ForeignContext { context, satp },
            address_space,
            // 初始化文件描述符表：预留 stdin(0)、stdout(1)、stderr(2)
            fd_table: vec![
                Some(Mutex::new(Arc::new(DiskFile::new(Arc::new(FileHandle::empty(true, false)))))),  // fd 0: stdin（可读）
                Some(Mutex::new(Arc::new(DiskFile::new(Arc::new(FileHandle::empty(false, true)))))),  // fd 1: stdout（可写）
                Some(Mutex::new(Arc::new(DiskFile::new(Arc::new(FileHandle::empty(false, true)))))),  // fd 2: stderr（可写）
            ],
            heap_bottom,
            program_brk: heap_bottom,
            allocator,
            stride: 0,
            priority: 16,
        })
    }

    /// 修改程序 break 位置（实现 sbrk 系统调用）
    pub fn change_program_brk(&mut self, size: isize) -> Option<usize> {
        let old_brk = self.program_brk;
        let new_brk = self.program_brk as isize + size;
        if new_brk < self.heap_bottom as isize {
            return None;
        }
        let new_brk = new_brk as usize;

        let old_brk_ceil = VAddr::<Sv39>::new(old_brk).ceil();
        let new_brk_ceil = VAddr::<Sv39>::new(new_brk).ceil();

        if size > 0 {
            if new_brk_ceil.val() > old_brk_ceil.val() {
                self.address_space
                    .map(old_brk_ceil..new_brk_ceil, &[], 0, build_flags("U_WRV"), MapVisibility::PRIVATE);
            }
        } else if size < 0 {
            if old_brk_ceil.val() > new_brk_ceil.val() {
                self.address_space.unmap(new_brk_ceil..old_brk_ceil);
            }
        }

        self.program_brk = new_brk;
        Some(old_brk)
    }
}
