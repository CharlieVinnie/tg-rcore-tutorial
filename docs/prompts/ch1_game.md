Now please implement the game of ch1 in tg-rcore-tutorial-ch1.

I have switched to a new branch in this chapter, so you can modify freely what you want.

The game of this chapter is to show the letters OS in Tangram. You can see the desired result under tg-rcore-tutorial-game-demo.

First, implement the user code that would be run by the OS in tg-rcore-tutorial-user/src/games. It should use open("/dev/fb0") to open the monitor, then use mmap to map the device into memory, then write into the mapped memory, and use ioctl(fd, FB_FLUSH, 0) to flush the screen. FB_FLUSH should be a constant of your choice.

Then, modify build.rs so that it compiles the user code into raw ASM, then insert it directly into main.rs using 

```
#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!(env!("APP_ASM")));
```

You can refer to ch2 for implementation details.

main.rs should print a simple boot message, then transfer to U-mode and run the user program. Upon the user program exiting, the OS should immediately shut down.

To support open("/dev/fb0") and mmap and ioctl, please configure open() to assert that the file name is "/dev/fb0" (because we don't have a legitimate file system yet), mmap should allocate the framebuffer and return its raw physical address, and ioctl should assert the FB_FLUSH constant and flush the framebuffer.