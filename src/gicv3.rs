use aarch64_cpu::registers::{MPIDR_EL1, Readable};
use alloc::boxed::Box;
use arm_gic_driver::DriverGeneric;
use arm_gic_driver::Interface;
use core::ptr::NonNull;
use kspin::SpinNoIrq;

use axplat::irq::{HandlerTable, IrqHandler, IrqIf};
use log::{debug, error, info, trace, warn};

/// The maximum number of IRQs.
const MAX_IRQ_COUNT: usize = 1024;

static IRQ_HANDLER_TABLE: HandlerTable<MAX_IRQ_COUNT> = HandlerTable::new();

static GICD: SpinNoIrq<Option<arm_gic_driver::v3::Gic>> = SpinNoIrq::new(None);
static GICR: SpinNoIrq<Option<Box<dyn arm_gic_driver::local::Interface>>> = SpinNoIrq::new(None);

struct IrqIfImpl;

pub(crate) fn init(gicd_vaddr: usize, gicr_vaddr: usize) {
    let mut gicd = arm_gic_driver::v3::Gic::new(
        NonNull::new(gicd_vaddr as *mut u8).unwrap(),
        NonNull::new(gicr_vaddr as *mut u8).unwrap(),
    );

    debug!("Initializing GICD at vaddr {:#x}", gicd_vaddr);
    gicd.open().unwrap();

    info!(
        "Initializing GICR for BSP. Global GICR vaddr at {:#x}",
        gicr_vaddr
    );
    let mut interface = gicd.cpu_local().unwrap();
    interface.open().unwrap();

    GICD.lock().replace(gicd);
    GICR.lock().replace(interface);
    info!("GIC initialized {}", current_cpu());
}

#[allow(dead_code)]
pub(crate) fn init_current_cpu() {
    debug!("Initializing GICR for current CPU {}", current_cpu());
    let mut interface = GICD.lock().as_mut().unwrap().cpu_local().unwrap();
    interface.open().unwrap();
    GICR.lock().replace(interface);
    debug!("Initialized GICR for current CPU {}", current_cpu());
}

fn current_cpu() -> usize {
    MPIDR_EL1.get() as usize & 0xffffff
}

pub(crate) fn set_enable(irq_num: usize, enabled: bool) {
    use arm_gic_driver::local::cap::ConfigLocalIrq;

    let mut gicd = GICD.lock();
    let d = gicd.as_mut().unwrap();

    if irq_num < 32 {
        trace!("GICR set enable: {} {}", irq_num, enabled);

        if enabled {
            d.get_gicr().irq_enable(irq_num.into()).unwrap();
        } else {
            d.get_gicr().irq_disable(irq_num.into()).unwrap();
        }
    } else {
        trace!("GICD set enable: {} {}", irq_num, enabled);
        if enabled {
            d.irq_enable(irq_num.into()).unwrap();
        } else {
            d.irq_disable(irq_num.into()).unwrap();
        }
    }
}

#[impl_plat_interface]
impl IrqIf for IrqIfImpl {
    /// Enables or disables the given IRQ.
    fn set_enable(irq_raw: usize, enabled: bool) {
        warn!("set_enable IRQ {} {}", irq_raw, enabled);
        set_enable(irq_raw, enabled);
    }

    /// Registers an IRQ handler for the given IRQ.
    ///
    /// It also enables the IRQ if the registration succeeds. It returns `false`
    /// if the registration failed.
    fn register(irq_num: usize, handler: IrqHandler) -> bool {
        trace!("register handler IRQ {}", irq_num);
        if IRQ_HANDLER_TABLE.register_handler(irq_num, handler) {
            Self::set_enable(irq_num, true);
            return true;
        }
        warn!("register handler for IRQ {} failed", irq_num);
        false
    }

    /// Unregisters the IRQ handler for the given IRQ.
    ///
    /// It also disables the IRQ if the unregistration succeeds. It returns the
    /// existing handler if it is registered, `None` otherwise.
    fn unregister(irq_num: usize) -> Option<IrqHandler> {
        trace!("unregister handler IRQ {}", irq_num);
        Self::set_enable(irq_num, false);
        IRQ_HANDLER_TABLE.unregister_handler(irq_num)
    }

    /// Handles the IRQ.
    ///
    /// It is called by the common interrupt handler. It should look up in the
    /// IRQ handler table and calls the corresponding handler. If necessary, it
    /// also acknowledges the interrupt controller after handling.
    fn handle(_unused: usize) {
        error!("Handling IRQ");
        let Some(irq) = GICR.lock().as_mut().unwrap().ack() else {
            return;
        };
        if !IRQ_HANDLER_TABLE.handle(irq.into()) {
            warn!("Unhandled IRQ {:?}", irq);
        }

        GICR.lock().as_mut().unwrap().eoi(irq);
        if GICR.lock().as_mut().unwrap().get_eoi_mode() {
            GICR.lock().as_mut().unwrap().dir(irq);
        }
    }

    /// Sends an inter-processor interrupt (IPI) to the specified target CPU or all CPUs.
    fn send_ipi(_irq_num: usize, _target: axplat::irq::IpiTarget) {
        todo!("send_ipi");
    }
}