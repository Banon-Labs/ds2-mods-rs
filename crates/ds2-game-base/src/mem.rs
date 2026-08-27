//! Tier A: fault-safe RAM readers + module base / RVA resolution.
//!
//! Implemented over raw `#[link(name = "kernel32")]` externs so this stays a zero-dependency
//! leaf that every DLL in this workspace can sit on without re-implementing
//! `ReadProcessMemory` reads.
//!
//! # Nothing in this file knows what game it is reading
//!
//! Every function below is Win32 or PE-format mechanics: a module handle, an integer range
//! test, a kernel-validated read that fails closed, a section-header walk. No address, no
//! offset and no structure layout appears here, and none may be added -- those live in
//! `ds2-rva`, which this crate deliberately does not depend on.
//!
//! Ported from `../er-mods-rs`'s `er-game-base::mem`. One function did NOT come across:
//! `vtable_in_game_image`, whose upper bound was a hardcoded `0x3000000` image span. That is a
//! measurement of a different game's binary (compare `ds2_rva::SIZE_OF_IMAGE`, which is where a
//! DS2 figure belongs), so importing it would have smuggled a wrong claim about this game into
//! a file that is supposed to make no claims at all. [`module_image_size`] reads the real span
//! out of the loaded image's own PE headers instead, and [`ptr_in_module`] takes it as an
//! argument.

use core::ffi::c_void;

/// `-1` cast to a handle: the current-process pseudo-handle accepted by
/// `ReadProcessMemory` without an `OpenProcess` round-trip.
const CURRENT_PROCESS_PSEUDO_HANDLE: isize = -1;
/// `ReadProcessMemory` returns a Win32 `BOOL`; zero means failure.
const RPM_FALSE: i32 = 0;
/// Init sentinel for the out-params / accumulators below. It is simply 0, named so the reads
/// read as `= ZERO` rather than as a bare literal that could be mistaken for an address.
const ZERO: usize = 0;

unsafe extern "system" {
    fn GetModuleHandleA(module_name: *const u8) -> isize;
    fn ReadProcessMemory(
        process: isize,
        base_address: *const c_void,
        buffer: *mut c_void,
        size: usize,
        bytes_read: *mut usize,
    ) -> i32;
}

/// Resolve the running game module's base address (`GetModuleHandleA(NULL)`).
///
/// `NULL` asks for the base of the process's own EXECUTABLE image, which -- for a DLL injected
/// into the game -- is the game exe and not the calling DLL.
///
/// Resolve it, never assume it. An image whose `DllCharacteristics` sets `DYNAMIC_BASE` is
/// relocated by the loader, so the live base is neither the preferred base recorded in the PE
/// header nor necessarily the same as it was last run. Whether a given build sets that bit is a
/// fact about that build, and belongs in `ds2-rva` rather than here.
pub fn game_module_base() -> Result<usize, String> {
    let module = unsafe { GetModuleHandleA(core::ptr::null()) };
    if module == 0 {
        return Err("failed to resolve game module: GetModuleHandleA(NULL) returned 0".to_string());
    }
    Ok(module as usize)
}

/// `game_module_base() + rva`.
pub fn game_rva(rva: u32) -> Result<usize, String> {
    Ok(game_module_base()? + rva as usize)
}

/// Cheap heap-pointer sanity check: above the low 64 KiB reserve and 8-byte aligned.
///
/// # Safety
///
/// There is NO precondition and no unsafety here: this function dereferences nothing.
/// It is integer arithmetic on `ptr` -- a range test against the low 64 KiB reserve and
/// an alignment mask -- and would be sound as a safe `fn`. The `unsafe` marker is
/// vestigial, kept so the signature matches the same function in `../er-mods-rs`, where
/// every DLL calls it -- a call site ported from there keeps its `unsafe` block and the
/// block keeps its meaning. A `true` result is a cheap plausibility screen, NOT proof that
/// `ptr` is mapped or points at a live object.
pub unsafe fn is_heap_aligned_ptr(ptr: usize) -> bool {
    const HEAP_LO: usize = 0x10000;
    const PTR_ALIGN_MASK: usize = 0x7;
    ptr >= HEAP_LO && (ptr & PTR_ALIGN_MASK) == ZERO
}

/// True if `ptr` falls inside the mapped span of a module loaded at `base`, i.e. within
/// `[base + 0x1000, base + size_of_image)`.
///
/// The lower bound skips the headers: `SectionAlignment` is 0x1000 on every image this will be
/// pointed at, so the first section starts one page in and nothing below that is code or data.
///
/// THE UPPER BOUND IS AN ARGUMENT ON PURPOSE. The `er-game-base` original hardcoded a
/// `0x3000000` fallback span, which is an Elden Ring image size; carrying it over would have
/// quietly widened the test past the end of a smaller image and still called it a bound. Pass
/// [`module_image_size`], which reads the real value from the loaded image's own PE headers. A
/// caller that cannot read them should fail, not guess.
///
/// A `true` result is a plausibility screen -- the pointer is in the right neighbourhood -- and
/// not proof that it is a vtable, a function, or even mapped.
pub fn ptr_in_module(ptr: usize, base: usize, size_of_image: usize) -> bool {
    const MODULE_MIN_OFFSET: usize = 0x1000;
    match base.checked_add(size_of_image) {
        Some(end) => ptr >= base.saturating_add(MODULE_MIN_OFFSET) && ptr < end,
        None => false,
    }
}

/// Fault-tolerant pointer-sized read: returns `None` on unmapped/freed memory
/// instead of raising an access violation.
///
/// # Safety
///
/// `addr` has NO precondition: any value, including 0, a freed pointer, or a wholly
/// unmapped address, is safe to pass. The read goes through `ReadProcessMemory`, which
/// validates the range in the kernel and returns `FALSE` rather than raising an access
/// violation, so this function cannot fault on a bad address -- that fault-tolerance is
/// the entire reason it exists.
///
/// What the CALLER owns is the meaning of the bytes that come back. A successful read
/// only proves those bytes were mapped at that instant; it does not prove they are a
/// live object of the expected type, and the game may free or overwrite the region on
/// another thread immediately afterwards. Treat the value as a sample, not a borrow.
///
/// The `unsafe` marker is therefore about interpretation, not memory validity. It is
/// retained rather than removed so the signature matches the same function in
/// `../er-mods-rs`: a call site ported from there keeps its `unsafe` block, and the block
/// keeps meaning "I own what these bytes are".
pub unsafe fn safe_read_usize(addr: usize) -> Option<usize> {
    let mut value: usize = ZERO;
    let mut read: usize = ZERO;
    let ok = unsafe {
        ReadProcessMemory(
            CURRENT_PROCESS_PSEUDO_HANDLE,
            addr as *const c_void,
            &mut value as *mut usize as *mut c_void,
            core::mem::size_of::<usize>(),
            &mut read,
        )
    };
    if ok != RPM_FALSE && read == core::mem::size_of::<usize>() {
        Some(value)
    } else {
        None
    }
}

/// Fault-tolerant i32 read (None on unmapped memory).
///
/// # Safety
///
/// `addr` has NO precondition: any value, including 0, a freed pointer, or a wholly
/// unmapped address, is safe to pass. The read goes through `ReadProcessMemory`, which
/// validates the range in the kernel and returns `FALSE` rather than raising an access
/// violation, so this function cannot fault on a bad address -- that fault-tolerance is
/// the entire reason it exists.
///
/// What the CALLER owns is the meaning of the bytes that come back. A successful read
/// only proves those bytes were mapped at that instant; it does not prove they are a
/// live object of the expected type, and the game may free or overwrite the region on
/// another thread immediately afterwards. Treat the value as a sample, not a borrow.
///
/// The `unsafe` marker is therefore about interpretation, not memory validity. It is
/// retained rather than removed so the signature matches the same function in
/// `../er-mods-rs`: a call site ported from there keeps its `unsafe` block, and the block
/// keeps meaning "I own what these bytes are".
pub unsafe fn safe_read_i32(addr: usize) -> Option<i32> {
    let mut value: i32 = 0;
    let mut read: usize = ZERO;
    let ok = unsafe {
        ReadProcessMemory(
            CURRENT_PROCESS_PSEUDO_HANDLE,
            addr as *const c_void,
            &mut value as *mut i32 as *mut c_void,
            core::mem::size_of::<i32>(),
            &mut read,
        )
    };
    if ok != RPM_FALSE && read == core::mem::size_of::<i32>() {
        Some(value)
    } else {
        None
    }
}

/// Fault-tolerant f32 read (None on unmapped memory).
///
/// # Safety
///
/// `addr` has NO precondition: any value, including 0, a freed pointer, or a wholly
/// unmapped address, is safe to pass. The read goes through `ReadProcessMemory`, which
/// validates the range in the kernel and returns `FALSE` rather than raising an access
/// violation, so this function cannot fault on a bad address -- that fault-tolerance is
/// the entire reason it exists.
///
/// What the CALLER owns is the meaning of the bytes that come back. A successful read
/// only proves those bytes were mapped at that instant; it does not prove they are a
/// live object of the expected type, and the game may free or overwrite the region on
/// another thread immediately afterwards. Treat the value as a sample, not a borrow.
///
/// The `unsafe` marker is therefore about interpretation, not memory validity. It is
/// retained rather than removed so the signature matches the same function in
/// `../er-mods-rs`: a call site ported from there keeps its `unsafe` block, and the block
/// keeps meaning "I own what these bytes are".
pub unsafe fn safe_read_f32(addr: usize) -> Option<f32> {
    let mut value: f32 = 0.0;
    let mut read: usize = ZERO;
    let ok = unsafe {
        ReadProcessMemory(
            CURRENT_PROCESS_PSEUDO_HANDLE,
            addr as *const c_void,
            &mut value as *mut f32 as *mut c_void,
            core::mem::size_of::<f32>(),
            &mut read,
        )
    };
    if ok != RPM_FALSE && read == core::mem::size_of::<f32>() {
        Some(value)
    } else {
        None
    }
}

/// Fault-tolerant single-byte read (None on unmapped memory).
///
/// # Safety
///
/// `addr` has NO precondition: any value, including 0, a freed pointer, or a wholly
/// unmapped address, is safe to pass. The read goes through `ReadProcessMemory`, which
/// validates the range in the kernel and returns `FALSE` rather than raising an access
/// violation, so this function cannot fault on a bad address -- that fault-tolerance is
/// the entire reason it exists.
///
/// What the CALLER owns is the meaning of the bytes that come back. A successful read
/// only proves those bytes were mapped at that instant; it does not prove they are a
/// live object of the expected type, and the game may free or overwrite the region on
/// another thread immediately afterwards. Treat the value as a sample, not a borrow.
///
/// The `unsafe` marker is therefore about interpretation, not memory validity. It is
/// retained rather than removed so the signature matches the same function in
/// `../er-mods-rs`: a call site ported from there keeps its `unsafe` block, and the block
/// keeps meaning "I own what these bytes are".
pub unsafe fn safe_read_u8(addr: usize) -> Option<u8> {
    let mut value: u8 = 0;
    let mut read: usize = ZERO;
    let ok = unsafe {
        ReadProcessMemory(
            CURRENT_PROCESS_PSEUDO_HANDLE,
            addr as *const c_void,
            &mut value as *mut u8 as *mut c_void,
            core::mem::size_of::<u8>(),
            &mut read,
        )
    };
    if ok != RPM_FALSE && read == core::mem::size_of::<u8>() {
        Some(value)
    } else {
        None
    }
}

/// Fault-tolerant bulk read into `out`. Returns true only if the whole slice was
/// read (the None-equivalent for byte buffers). This is what lets a signature scan walk
/// `.text` and fail closed on a drifted or unmapped region instead of faulting.
///
/// # Safety
///
/// `addr` has NO precondition -- see [`safe_read_usize`]; the read is performed by
/// `ReadProcessMemory` and fails closed on an unmapped range instead of faulting. This
/// is what lets a scan walk a drifted or partially-unmapped image without crashing the
/// game.
///
/// `out` is an ordinary Rust slice and is only written on a fully successful read, so a
/// `false` return leaves its contents unspecified but initialised. The caller owns the
/// meaning of the bytes, exactly as above.
pub unsafe fn read_bytes(addr: usize, out: &mut [u8]) -> bool {
    if out.is_empty() {
        return true;
    }
    let mut read: usize = ZERO;
    let ok = unsafe {
        ReadProcessMemory(
            CURRENT_PROCESS_PSEUDO_HANDLE,
            addr as *const c_void,
            out.as_mut_ptr() as *mut c_void,
            out.len(),
            &mut read,
        )
    };
    ok != RPM_FALSE && read == out.len()
}

/// Locate the `IMAGE_NT_HEADERS` of the module mapped at `base`, fault-safely.
///
/// Returns the address of the `PE\0\0` signature, or `None` if the headers are unreadable or
/// the signature is absent. Every read goes through [`read_bytes`], so pointing this at an
/// address that is not a mapped image returns `None` rather than faulting.
fn nt_headers(base: usize) -> Option<usize> {
    // DOS header: e_lfanew (u32) at +0x3C -> PE header offset.
    let mut w4 = [0u8; 4];
    if !unsafe { read_bytes(base + 0x3C, &mut w4) } {
        return None;
    }
    let pe = base.checked_add(u32::from_le_bytes(w4) as usize)?;
    let mut sig = [0u8; 4];
    if !unsafe { read_bytes(pe, &mut sig) } || &sig != b"PE\0\0" {
        return None;
    }
    Some(pe)
}

/// The running game image's `SizeOfImage`: how many bytes the loader mapped, read out of the
/// in-memory PE headers rather than assumed.
///
/// This is the upper bound to hand [`ptr_in_module`]. It is read at runtime because it is a
/// property of the build that is actually loaded -- a game update changes it, and a constant
/// copied from a different game is simply a wrong number wearing a type.
///
/// `SizeOfImage` sits at optional-header `+0x38` in both PE32 and PE32+: `ImageBase` widens
/// from 4 to 8 bytes at `+0x18`, but PE32 has a `BaseOfData` field there that PE32+ drops, so
/// every field from `SectionAlignment` (`+0x20`) onward is at the same offset in both. No magic
/// check is needed to read it.
pub fn module_image_size() -> Option<usize> {
    let base = game_module_base().ok()?;
    let pe = nt_headers(base)?;
    let mut size = [0u8; 4];
    // Optional header begins at pe+24 (4-byte signature + 20-byte COFF file header).
    if !unsafe { read_bytes(pe + 24 + 0x38, &mut size) } {
        return None;
    }
    let size = u32::from_le_bytes(size) as usize;
    (size != ZERO).then_some(size)
}

/// Resolve the running game image's `.text` section as `(start_va, len)` by parsing
/// the in-memory PE headers. Returns `None` if the headers are unreadable or no
/// `.text` section is found. This is the bound for a fault-safe AOB scan; it makes
/// signature-based function discovery version-agnostic (no hardcoded RVAs).
pub fn module_text_range() -> Option<(usize, usize)> {
    let base = game_module_base().ok()?;
    let pe = nt_headers(base)?;
    unsafe {
        // COFF file header at pe+4: NumberOfSections (u16) at +2, SizeOfOptionalHeader (u16) at +16.
        let mut nsec = [0u8; 2];
        let mut optsz = [0u8; 2];
        if !read_bytes(pe + 6, &mut nsec) || !read_bytes(pe + 20, &mut optsz) {
            return None;
        }
        let num_sections = u16::from_le_bytes(nsec) as usize;
        let opt_size = u16::from_le_bytes(optsz) as usize;
        // Section headers (40 bytes each) begin after the optional header.
        let mut sec = pe + 24 + opt_size;
        for _ in 0..num_sections.min(96) {
            let mut hdr = [0u8; 40];
            if !read_bytes(sec, &mut hdr) {
                return None;
            }
            // name[0..8], VirtualSize[8..12], VirtualAddress[12..16].
            if &hdr[0..8] == b".text\0\0\0" {
                let vsize = u32::from_le_bytes([hdr[8], hdr[9], hdr[10], hdr[11]]) as usize;
                let vaddr = u32::from_le_bytes([hdr[12], hdr[13], hdr[14], hdr[15]]) as usize;
                if vaddr == 0 || vsize == 0 {
                    return None;
                }
                return Some((base + vaddr, vsize));
            }
            sec += 40;
        }
        None
    }
}

/// Fault-tolerant u16 read (None on unmapped memory).
///
/// # Safety
///
/// `addr` has NO precondition: any value, including 0, a freed pointer, or a wholly
/// unmapped address, is safe to pass. The read goes through `ReadProcessMemory`, which
/// validates the range in the kernel and returns `FALSE` rather than raising an access
/// violation, so this function cannot fault on a bad address -- that fault-tolerance is
/// the entire reason it exists.
///
/// What the CALLER owns is the meaning of the bytes that come back. A successful read
/// only proves those bytes were mapped at that instant; it does not prove they are a
/// live object of the expected type, and the game may free or overwrite the region on
/// another thread immediately afterwards. Treat the value as a sample, not a borrow.
///
/// The `unsafe` marker is therefore about interpretation, not memory validity. It is
/// retained rather than removed so the signature matches the same function in
/// `../er-mods-rs`: a call site ported from there keeps its `unsafe` block, and the block
/// keeps meaning "I own what these bytes are".
pub unsafe fn safe_read_u16(addr: usize) -> Option<u16> {
    let mut value: u16 = 0;
    let mut read: usize = ZERO;
    let ok = unsafe {
        ReadProcessMemory(
            CURRENT_PROCESS_PSEUDO_HANDLE,
            addr as *const c_void,
            &mut value as *mut u16 as *mut c_void,
            core::mem::size_of::<u16>(),
            &mut read,
        )
    };
    if ok != RPM_FALSE && read == core::mem::size_of::<u16>() {
        Some(value)
    } else {
        None
    }
}

/// Page granularity for [`safe_read_cstr`]'s walk. Mapping is per-page, so a page is either
/// readable in full or not at all -- which is what makes a page-bounded chunk the largest read
/// that cannot fail merely because of where the string happens to sit.
const PAGE_SIZE: usize = 0x1000;

/// Fault-safe, LENGTH-BOUNDED read of a NUL-terminated C string.
///
/// This exists because `CStr::from_ptr` on a pointer that came from outside our own code is a
/// crash waiting for the right afternoon: it calls `strlen`, `strlen` dereferences, and a
/// non-null-but-garbage pointer takes the process down. A null check does not help -- the
/// pointers that actually took Elden Ring down on two testers in `../er-mods-rs` on
/// 2026-08-23 were `0x011000010e05acda` and `0x0110000107be5e2c`, both very much non-null.
/// Nothing about that failure was game-specific: it was a foreign pointer handed to `strlen`.
///
/// Returns the bytes BEFORE the NUL, or `None` if the string is unreadable, if `addr` is null,
/// or if no NUL appears within `max_len`. That last case is deliberately a failure and not a
/// truncation: a readable region with no terminator in range is not a string we have any reason
/// to trust, and silently returning `max_len` bytes of it would launder junk into a value the
/// caller goes on to use.
///
/// # Safety
///
/// `addr` has NO precondition -- see [`safe_read_usize`]. Every read goes through
/// `ReadProcessMemory`, which fails closed on an unmapped page instead of faulting. The reads
/// are page-bounded so a legitimate string ending in the last mapped page of a region is not
/// rejected just because the next page is absent.
///
/// The caller still owns the MEANING of the bytes: a successful read proves only that they were
/// mapped at that instant.
pub unsafe fn safe_read_cstr(addr: usize, max_len: usize) -> Option<Vec<u8>> {
    cstr_walk(addr, max_len, &mut |at, out| unsafe { read_bytes(at, out) })
}

/// The page-bounded walk behind [`safe_read_cstr`], with the actual read injected.
///
/// Split out for one reason: `ReadProcessMemory` does not exist on the host, so a test that
/// called [`safe_read_cstr`] would not link, and the guard would ship with its logic unexercised
/// -- which is how the bug it exists to prevent got out in the first place. `read` stands in for
/// the kernel: it returns `false` for a range that is not fully readable, exactly as
/// [`read_bytes`] does.
fn cstr_walk(
    addr: usize,
    max_len: usize,
    read: &mut dyn FnMut(usize, &mut [u8]) -> bool,
) -> Option<Vec<u8>> {
    if addr == ZERO || max_len == ZERO {
        return None;
    }
    let mut out: Vec<u8> = Vec::new();
    let mut cursor = addr;
    while out.len() < max_len {
        // Never let one read span a page boundary. `ReadProcessMemory` is all-or-nothing, so a
        // read that ran on into an unmapped neighbouring page would report failure for a string
        // that is entirely present in the page it started in.
        let to_page_end = PAGE_SIZE - (cursor & (PAGE_SIZE - 1));
        let want = to_page_end.min(max_len - out.len());
        let mut chunk = vec![0u8; want];
        if !read(cursor, &mut chunk) {
            return None;
        }
        if let Some(nul) = chunk.iter().position(|&byte| byte == 0) {
            out.extend_from_slice(&chunk[..nul]);
            return Some(out);
        }
        out.extend_from_slice(&chunk);
        cursor = cursor.checked_add(want)?;
    }
    None
}

#[cfg(test)]
mod cstr_tests {
    use super::{PAGE_SIZE, cstr_walk};

    /// The two pointers that actually took Elden Ring down on 2026-08-23 in `../er-mods-rs`.
    /// Both non-null, which is the entire point: the guard they defeated was a null check. They
    /// are carried over as test vectors because the failure they caused is a property of
    /// `strlen`, not of that game.
    const CRASH_POINTERS: [usize; 2] = [0x0110_0001_0e05_acda, 0x0110_0001_07be_5e2c];

    /// A reader that models ONE mapped page at `base` whose contents start with `bytes`;
    /// everything outside that page is unmapped. The page is a full [`PAGE_SIZE`] because that is
    /// what mapping granularity means -- a stub page shorter than that would reject the
    /// page-bounded chunk the walk legitimately asks for, and test the stub rather than the code.
    fn one_page(base: usize, bytes: &[u8]) -> impl FnMut(usize, &mut [u8]) -> bool {
        let mut page = vec![0xffu8; PAGE_SIZE];
        page[..bytes.len()].copy_from_slice(bytes);
        move |at, out| {
            let Some(start) = at.checked_sub(base) else {
                return false;
            };
            let Some(end) = start.checked_add(out.len()) else {
                return false;
            };
            if end > page.len() {
                return false;
            }
            out.copy_from_slice(&page[start..end]);
            true
        }
    }

    #[test]
    fn an_unreadable_pointer_is_refused_rather_than_dereferenced() {
        for bad in CRASH_POINTERS {
            let mut never = |_: usize, _: &mut [u8]| false;
            assert_eq!(
                cstr_walk(bad, 255, &mut never),
                None,
                "a garbage non-null pointer must fail closed, not walk memory"
            );
        }
    }

    #[test]
    fn null_is_refused_before_any_read_is_attempted() {
        let mut reads = 0usize;
        let mut counting = |_: usize, _: &mut [u8]| {
            reads += 1;
            true
        };
        assert_eq!(cstr_walk(0, 255, &mut counting), None);
        assert_eq!(reads, 0, "null must short-circuit, not reach the reader");
    }

    #[test]
    fn a_terminated_string_comes_back_without_its_nul() {
        let page = b"lobby_key\0trailing junk".to_vec();
        let mut read = one_page(0x1_0000, &page);
        assert_eq!(
            cstr_walk(0x1_0000, 255, &mut read).as_deref(),
            Some(&b"lobby_key"[..])
        );
    }

    #[test]
    fn a_run_with_no_terminator_in_range_is_refused_not_truncated() {
        // one_page pads with 0xff, so nothing in range is a NUL.
        let mut read = one_page(0x1_0000, &[b'A'; 512]);
        assert_eq!(
            cstr_walk(0x1_0000, 64, &mut read),
            None,
            "no NUL within max_len is junk, and truncating it would launder junk into a value"
        );
    }

    #[test]
    fn a_string_at_the_end_of_a_mapped_page_survives_the_unmapped_neighbour() {
        // The string sits in the last bytes of the page, so a read that ran past the page end
        // would fail. Page-bounded chunking is what keeps this case readable.
        let tail = b"lobby_key\0";
        let mut page = vec![b'.'; PAGE_SIZE];
        page[PAGE_SIZE - tail.len()..].copy_from_slice(tail);
        let base = 0x1_0000;
        let mut read = one_page(base, &page);
        let at = base + PAGE_SIZE - tail.len();
        assert_eq!(
            cstr_walk(at, 255, &mut read).as_deref(),
            Some(&b"lobby_key"[..])
        );
    }
}

#[cfg(test)]
mod span_tests {
    use super::{is_heap_aligned_ptr, ptr_in_module};

    /// The bound this function replaced was a hardcoded Elden Ring image span, and the failure
    /// mode of getting it wrong is silent: a pointer past the end of a smaller image is accepted
    /// and the caller believes it screened one. So the end of the span is asserted explicitly.
    #[test]
    fn the_span_ends_where_the_caller_says_it_does() {
        let base = 0x1_4000_0000;
        // An arbitrary span, smaller than the `0x0300_0000` the ER original hardcoded. It is
        // not any particular game's `SizeOfImage`, and must not become one -- a real image size
        // in this crate would be exactly the game knowledge the crate is built to exclude.
        let size = 0x0100_0000;
        assert!(ptr_in_module(base + 0x1000, base, size), "first section");
        assert!(ptr_in_module(base + size - 1, base, size), "last byte");
        assert!(
            !ptr_in_module(base + size, base, size),
            "one past the end is outside the image, not inside it"
        );
        assert!(
            !ptr_in_module(base + 0x0300_0000, base, size),
            "the ER fallback span reached well past the end of a smaller image"
        );
    }

    /// The headers are not code or data; nothing that belongs to a section lives below the
    /// first page.
    #[test]
    fn the_pe_headers_are_below_the_span() {
        let base = 0x1_4000_0000;
        let size = 0x100_000;
        assert!(!ptr_in_module(base, base, size), "the DOS header");
        assert!(!ptr_in_module(base + 0xfff, base, size), "still headers");
        assert!(!ptr_in_module(0, base, size), "null");
    }

    /// An absurd span must not wrap the address space and start accepting everything.
    #[test]
    fn an_overflowing_span_refuses_rather_than_wrapping() {
        assert!(!ptr_in_module(0x1000, usize::MAX - 0x10, usize::MAX));
    }

    /// Integer arithmetic only -- see the function's own safety note. The `unsafe` block is
    /// here because the signature is kept identical to the sibling repo's, not because
    /// anything is dereferenced.
    #[test]
    fn heap_alignment_screens_low_and_misaligned_pointers() {
        unsafe {
            assert!(!is_heap_aligned_ptr(0), "null");
            assert!(
                !is_heap_aligned_ptr(0xffff),
                "inside the low 64 KiB reserve"
            );
            assert!(!is_heap_aligned_ptr(0x10004), "not 8-byte aligned");
            assert!(is_heap_aligned_ptr(0x10000));
            assert!(is_heap_aligned_ptr(0x1_4000_0008));
        }
    }
}
