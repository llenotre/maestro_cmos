//! This module handles CMOS features on IBM PC.
//! The CMOS is a persistent memory used to store some BIOS settings.

#![no_std]

extern crate kernel;

mod rtc;

use core::ops::Range;
use kernel::acpi;
use kernel::errno::Errno;
use kernel::idt;
use kernel::io;
use kernel::module::version::Version;
use kernel::time::ClockSource;
use kernel::time::unit::Timestamp;
use kernel::time::unit::TimestampScale;
use kernel::time;

// cmos module, version 1.0.0
kernel::module!("cmos", Version::new(1, 0, 0));

/// The ID of the port used to select the CMOS register to read.
const SELECT_PORT: u16 = 0x70;
/// The ID of the port to read or write a CMOS port previously selected.
const VALUE_PORT: u16 = 0x71;

/// The ID of the register storing the current time second.
const SECOND_REGISTER: u8 = 0x00;
/// The ID of the register storing the current time minute.
const MINUTE_REGISTER: u8 = 0x02;
/// The ID of the register storing the current time hour.
const HOUR_REGISTER: u8 = 0x04;
/// The ID of the register storing the current time day of month.
const DAY_OF_MONTH_REGISTER: u8 = 0x07;
/// The ID of the register storing the current time month.
const MONTH_REGISTER: u8 = 0x08;
/// The ID of the register storing the current time year.
const YEAR_REGISTER: u8 = 0x09;
/// The ID of the register storing the current time century.
const CENTURY_REGISTER: u8 = 0x32;

/// The ID of the status register A.
const STATUS_A_REGISTER: u8 = 0x0a;
/// The ID of the status register B.
const STATUS_B_REGISTER: u8 = 0x0b;
/// The ID of the status register C.
const STATUS_C_REGISTER: u8 = 0x0c;

/// Bit of status register A, tells whether the time is being updated.
const UPDATE_FLAG: u8 = 1 << 7;
/// Bit of status register B, tells whether the 24 hour format is set.
const FORMAT_24_FLAG: u8 = 1 << 1;
/// Bit of status register B, tells whether binary mode is set.
const FORMAT_BCD_FLAG: u8 = 1 << 2;

/// The ID of the register used to store the Floopy Drive type.
const FLOPPY_DRIVE_REGISTER: u8 = 0x10;

/// Reads the register `reg` and returns the value.
fn read(reg: u8) -> u8 {
	unsafe {
		io::outb(SELECT_PORT, (1 << 7) | reg);
		io::inb(VALUE_PORT)
	}
}

/// Enumeration representing a drive type.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FloppyDriveType {
	/// No drive is present.
	NoDrive,
	/// 360KB, 5.25 inches
	Type360kb525,
	/// 1200KB, 5.25 inches
	Type1200kb525,
	/// 720KB, 3.5 inches
	Type720kb350,
	/// 1440KB, 3.5 inches
	Type1440kb350,
	/// 2880KB, 3.5 inches
	Type2880kb350,
}

/// Structure representing the state of the floppy drives.
pub struct FloppyDrives {
	/// The type of the master floppy drive.
	master: FloppyDriveType,
	/// The type of the slave floppy drive.
	slave: FloppyDriveType,
}

impl FloppyDrives {
	/// Returns the type of the master floppy driver.
	pub fn get_master_type(&self) -> &FloppyDriveType {
		&self.master
	}

	/// Returns the type of the slave floppy driver.
	pub fn get_slave_type(&self) -> &FloppyDriveType {
		&self.slave
	}
}

/// Converts the given number to the associated floppy type.
fn floppy_type_from_number(n: u8) -> FloppyDriveType {
	match n {
		1 => FloppyDriveType::Type360kb525,
		2 => FloppyDriveType::Type1200kb525,
		3 => FloppyDriveType::Type720kb350,
		4 => FloppyDriveType::Type1440kb350,
		5 => FloppyDriveType::Type2880kb350,
		_ => FloppyDriveType::NoDrive,
	}
}

/// Returns the state of the floppy drives.
pub fn get_floppy_type() -> FloppyDrives {
	let floppy_state = read(FLOPPY_DRIVE_REGISTER);
	let master_state = (floppy_state >> 4) & 0xf;
	let slave_state = floppy_state & 0xf;

	FloppyDrives {
		master: floppy_type_from_number(master_state),
		slave: floppy_type_from_number(slave_state),
	}
}

/// Tells whether the CMOS is ready for time reading.
fn is_time_ready() -> bool {
	read(STATUS_A_REGISTER) & UPDATE_FLAG == 0
}

/// Waits for the CMOS to be ready for reading the time.
fn time_wait() {
	while is_time_ready() {}
	while !is_time_ready() {}
}

/// Tells whether the given year is a leap year or not.
fn is_leap_year(year: u32) -> bool {
	if year % 4 != 0 {
		false
	} else if year % 100 != 0 {
		true
	} else {
		year % 400 == 0
	}
}

/// Returns the number of leap years between the two years.
/// `y0` and `y1` are the range in years.
/// undefined.
fn leap_years_between(range: Range<u32>) -> u32 {
	range.into_iter()
		.filter(| year | is_leap_year(*year))
		.count() as _
}

/// Returns the number of days since epoch from the year, month and day of the month.
fn get_days_since_epoch(year: u32, month: u32, day: u32) -> u32 {
	let year_days = (year - 1970) * 365 + leap_years_between(1970..year);

	let mut month_days = (((month + 1) / 2) * 31) + ((month / 2) * 30);
	if is_leap_year(year) && month >= 2 {
		month_days += 1;
	}

	year_days + month_days + day
}

/// Structure representing the CMOS clock source.
/// This source is really slow to initialize (waits up to 1 second before reading).
/// Maskable interrupts are disabled when retrieving the timestamp.
pub struct CMOSClock {
	/// Tells whether the century register is available.
	century_register: bool,

	/// The clock's current timestamp. If None, the clock is not initialized.
	timestamp: Option<Timestamp>,
}

impl CMOSClock {
	/// Creates a new instance. `century_register` tells whether the century register is available.
	/// If it isn't available, the 21st century is assumed.
	pub fn new(century_register: bool) -> Self {
		Self {
			century_register,

			timestamp: None,
		}
	}

	/// Tells whether the century register is available.
	pub fn has_century_register(&self) -> bool {
		self.century_register
	}

	/// Initializes the clock. If already initialized, the function does nothing.
	fn init(&mut self) {
		if self.timestamp.is_some() {
			return;
		}

		idt::wrap_disable_interrupts(|| {
			time_wait();
			let mut second = read(SECOND_REGISTER) as u32;
			let mut minute = read(MINUTE_REGISTER) as u32;
			let mut hour = read(HOUR_REGISTER) as u32;
			let mut day = read(DAY_OF_MONTH_REGISTER) as u32;
			let mut month = read(MONTH_REGISTER) as u32;
			let mut year = read(YEAR_REGISTER) as u32;
			let mut century = if self.century_register {
				read(CENTURY_REGISTER)
			} else {
				20
			} as u32;

			let status_b = read(STATUS_B_REGISTER);
			if status_b & FORMAT_BCD_FLAG == 0 {
				second = (second & 0x0f) + ((second / 16) * 10);
				minute = (minute & 0x0f) + ((minute / 16) * 10);
				hour = ((hour & 0x0f) + (((hour & 0x70) / 16) * 10)) | (hour & 0x80);
				day = (day & 0x0f) + ((day / 16) * 10);
				month = (month & 0x0f) + ((month / 16) * 10);
				year = (year & 0x0f) + ((year / 16) * 10);
				if self.century_register {
					century = (century & 0x0f) + (century / 16) * 10;
				}
			}

			if (status_b & FORMAT_24_FLAG) == 0 && (hour & 0x80) != 0 {
				hour = ((hour & 0x7f) + 12) % 24;
			}

			day -= 1;
			month -= 1;
			year += century * 100;

			let days_since_epoch = get_days_since_epoch(year, month, day);
			self.timestamp = Some((days_since_epoch * 86400) as u64
				+ (hour * 3600) as u64
				+ (minute * 60) as u64
				+ second as u64);
		});
	}
}

impl ClockSource for CMOSClock {
	fn get_name(&self) -> &'static str {
		"cmos"
	}

	fn get_time(&mut self, scale: TimestampScale) -> Timestamp {
		if self.timestamp.is_none() {
			self.init();
		}

		TimestampScale::convert(self.timestamp.unwrap(), TimestampScale::Second, scale)
	}
}

fn init_() -> Result<(), Errno> {
	// Creating and adding the CMOS clock
	let cmos_clock = CMOSClock::new(acpi::is_century_register_present());
	time::add_clock_source(cmos_clock)?;

	rtc::init()
}

#[no_mangle]
pub extern "C" fn init() -> bool {
	if let Err(e) = init_() {
		kernel::println!("Failed to create CMOS clock source: {}", e);
		false
	} else {
		true
	}
}

#[no_mangle]
pub extern "C" fn fini() {
	rtc::fini();

	// Removing the CMOS clock
	time::remove_clock_source("CMOS");
}
