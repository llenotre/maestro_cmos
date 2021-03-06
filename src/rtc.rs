//! The Real Time Clock (RTC) is the clock used by the CMOS to maintain system time.

use kernel::errno::Errno;
use kernel::event::CallbackHook;
use kernel::event::InterruptResult;
use kernel::event::InterruptResultAction;
use kernel::event;
use kernel::idt;
use kernel::io;

/// Global variable storing the RTC clock handle hook.
static mut RTC_HANDLE: Option<CallbackHook> = None;
/// The interruption counter.
static mut COUNTER: usize = 0;

/// Reset register C of the CMOS to allow the next RTC interrupt.
fn reset() {
	unsafe {
		io::outb(super::SELECT_PORT, super::STATUS_C_REGISTER);
		io::inb(super::VALUE_PORT);
	}
}

/// Initializes the RTC.
pub fn init() -> Result<(), Errno> {
	idt::wrap_disable_interrupts(|| {
		// Enable RTC
		unsafe {
			io::outb(super::SELECT_PORT, super::STATUS_B_REGISTER | 0x80);
			let prev = io::inb(super::VALUE_PORT);
			io::outb(super::SELECT_PORT, super::STATUS_B_REGISTER | 0x80);
			io::outb(super::VALUE_PORT, prev | 0x40);
		}

		// Set frequency to 8 kHz
		unsafe {
			io::outb(0x70, super::STATUS_A_REGISTER | 0x80);
			let prev = io::inb(super::VALUE_PORT);
			io::outb(0x70, super::STATUS_A_REGISTER | 0x80);
			io::outb(0x71, (prev & 0xF0) | 3);
		}
	});

	// Registering callback
	let handle = event::register_callback(0x08, 0, | _, _, _, _ | {
		let mut counter = unsafe {
			COUNTER
		};

		counter += 1;
		if counter == 8 {
			// TODO Incremenet 1ms

			counter = 0;
		}

		unsafe {
			COUNTER = counter;
		}

		reset();
		InterruptResult::new(false, InterruptResultAction::Resume)
	})?;

	// Safe because the function is call only once at initialization
	unsafe {
		RTC_HANDLE = Some(handle);
	}

	Ok(())
}

/// Disables the RTC.
pub fn fini() {
	idt::wrap_disable_interrupts(|| unsafe {
		io::outb(super::SELECT_PORT, super::STATUS_B_REGISTER | 0x80);
		let prev = io::inb(super::VALUE_PORT);
		io::outb(super::SELECT_PORT, super::STATUS_B_REGISTER | 0x80);
		io::outb(super::VALUE_PORT, prev & !0x40);
	});

	// Safe because the function is call only once at initialization
	unsafe {
		RTC_HANDLE = None;
	}
}
