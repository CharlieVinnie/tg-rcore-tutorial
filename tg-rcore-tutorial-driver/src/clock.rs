pub fn get_time_us() -> usize {
    riscv::register::time::read() * 10000 / 125
}