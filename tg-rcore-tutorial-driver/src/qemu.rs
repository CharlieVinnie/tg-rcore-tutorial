use crate::{plic::{IntrTargetPriority, PLIC}, address::VIRT_PLIC};

pub const HART_ID: usize = 0;

pub fn setup_interrupt_for(slot: usize) {
    let mut plic = unsafe { PLIC::new(VIRT_PLIC) };
    plic.enable(HART_ID, IntrTargetPriority::Supervisor, slot);
    plic.set_priority(slot, 1);
}

pub fn enable_external_interrupts() {
    use riscv::register::sie;

    let mut plic = unsafe { PLIC::new(VIRT_PLIC) };
    plic.set_threshold(HART_ID, IntrTargetPriority::Supervisor, 0);
    plic.set_threshold(HART_ID, IntrTargetPriority::Machine, 1);

    unsafe {
        sie::set_sext();
    }
}

pub fn claim_irq() -> Option<u32> {
    let mut plic = unsafe { PLIC::new(VIRT_PLIC) };
    let irq = plic.claim(HART_ID, IntrTargetPriority::Supervisor);
    // Phantom interrupt: the interrupt might disappear before we claim it
    if irq == 0 { None } else { Some(irq) }
}

pub fn complete_irq(irq: u32) {
    let mut plic = unsafe { PLIC::new(VIRT_PLIC) };
    plic.complete(HART_ID, IntrTargetPriority::Supervisor, irq);
}