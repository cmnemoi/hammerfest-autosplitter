//! Sonde compilee par runtime_probe.py. Ne lit que le processus de test.
#![no_std]

const PID: u64 = __PROBE_PID__;
const CONTROL: u64 = __PROBE_CONTROL__;

#[link(wasm_import_module = "env")]
extern "C" {
    fn process_attach_by_pid(pid: u64) -> u64;
    fn process_detach(process: u64);
    fn process_read(process: u64, addr: u64, buf: *mut u8, len: usize) -> bool;
    fn process_get_memory_range_count(process: u64) -> u64;
    fn process_get_memory_range_address(process: u64, index: u64) -> u64;
    fn process_get_memory_range_size(process: u64, index: u64) -> u64;
    fn process_get_memory_range_flags(process: u64, index: u64) -> u64;
    fn timer_set_variable(key: *const u8, key_len: usize, value: *const u8, value_len: usize);
    fn runtime_set_tick_rate(rate: f64);
}

#[link(wasm_import_module = "wasi_snapshot_preview1")]
extern "C" {
    fn clock_time_get(id: u32, precision: u64, result: *mut u64) -> u16;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { core::arch::wasm32::unreachable() }

unsafe fn now() -> u64 {
    let mut t = 0;
    assert_eq!(clock_time_get(1, 1, &mut t), 0);
    t
}

unsafe fn variable(key: &str, mut value: u64) {
    let mut buf = [0u8; 20];
    let mut at = buf.len();
    loop {
        at -= 1;
        buf[at] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 { break; }
    }
    timer_set_variable(key.as_ptr(), key.len(), buf[at..].as_ptr(), buf.len() - at);
}

unsafe fn mapped(process: u64, target: u64) -> bool {
    for i in 0..process_get_memory_range_count(process) {
        let base = process_get_memory_range_address(process, i);
        let size = process_get_memory_range_size(process, i);
        if base <= target && target - base < size {
            return process_get_memory_range_flags(process, i) & 2 != 0;
        }
    }
    false
}

static mut CONTROL_PROCESS: u64 = 0;
static mut CACHED_PROCESS: u64 = 0;
static mut TRIAL: u64 = 0;
static mut START: u64 = 0;
static mut NEXT_FRESH: u64 = 0;
static mut DIRECT: bool = false;
static mut CACHED: bool = false;
static mut FRESH: bool = false;

#[no_mangle]
pub unsafe extern "C" fn update() {
    if CONTROL_PROCESS == 0 {
        runtime_set_tick_rate(120.0);
        CONTROL_PROCESS = process_attach_by_pid(PID);
        if CONTROL_PROCESS == 0 { return; }
    }
    let mut command = [0u64; 2];
    if !process_read(CONTROL_PROCESS, CONTROL, command.as_mut_ptr().cast(), 16) { return; }
    let [trial, target] = command;
    if trial == 0 { return; }
    if trial != TRIAL {
        if CACHED_PROCESS != 0 { process_detach(CACHED_PROCESS); }
        CACHED_PROCESS = process_attach_by_pid(PID);
        if CACHED_PROCESS == 0 { return; }
        assert!(!mapped(CACHED_PROCESS, target));
        START = now();
        NEXT_FRESH = START;
        DIRECT = false;
        CACHED = false;
        FRESH = false;
        TRIAL = trial;
        variable("direct_us", 0);
        variable("cached_us", 0);
        variable("fresh_us", 0);
        variable("ready", trial);
        return;
    }
    if !DIRECT {
        let mut byte = 0;
        if process_read(CONTROL_PROCESS, target, &mut byte, 1) && byte == 0x5a {
            variable("direct_us", (now() - START) / 1000);
            DIRECT = true;
        }
    }
    if !CACHED && mapped(CACHED_PROCESS, target) {
        variable("cached_us", (now() - START) / 1000);
        CACHED = true;
    }
    if !FRESH && now() >= NEXT_FRESH {
        let fresh = process_attach_by_pid(PID);
        if fresh != 0 {
            if mapped(fresh, target) {
                variable("fresh_us", (now() - START) / 1000);
                FRESH = true;
            }
            process_detach(fresh);
        }
        NEXT_FRESH = now() + 100_000_000;
    }
    if DIRECT && CACHED && FRESH { variable("done", trial); }
}
