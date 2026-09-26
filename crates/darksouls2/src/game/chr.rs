//! The character controllers: `CharacterCtrlBase`, `CharacterCtrl` and `PlayerCtrl`.
//!
//! Each class embeds its parent at offset zero, so a `PlayerCtrl` is a `CharacterCtrl` is a
//! `CharacterCtrlBase` by field access as well as by DLRF's parent chain. All three are
//! registered with DLRF, and the size each registration reports is the size the game allocates
//! before running the matching constructor, which is what the size tests below pin.
//!
//! Only the fields a crate in this repo reads are named; the rest is padding named by offset.
//! These objects are never built or copied here: they are reached as pointers into the game's
//! memory, the local player's through `GameManagerImp::player_ctrl`.

use core::ffi::c_void;
use core::ptr::NonNull;

/// `CharacterCtrlBase`, the root of the controller chain.
///
/// Source of name: RTTI and DLRF. Nothing past the vtable is read by this repo yet.
#[repr(C)]
pub struct CharacterCtrlBase {
    /// The most derived class's vtable. Comparing it with a class's vtable RVA in `ds2-rva` is an
    /// exact test of which controller an object is.
    pub vtable: *const c_void,
    _unk0008: [u8; 0x50],
}

/// `CharacterCtrl`, the controller every character in the roster is, or derives from.
///
/// Source of name: RTTI and DLRF.
#[repr(C)]
pub struct CharacterCtrl {
    /// The `CharacterCtrlBase` part.
    pub base: CharacterCtrlBase,
    _unk0058: [u8; 0x38],
    /// World position as x, y, z and a `w` nothing here reads. Y is up.
    ///
    /// The vtable's position getter copies these four floats and does nothing else; `PlayerCtrl`
    /// inherits that getter unchanged.
    pub position: [f32; 4],
    _unk00a0: [u8; 0x10],
    /// The phantom block `CharacterCtrl::assignPhantomProperties` allocates and stores, whose
    /// phantom param id decides whether a player is a person or a replay.
    pub phantom_block: Option<NonNull<PhantomBlock>>,
    _unk00b8: [u8; 0x60],
    /// The factory's label for what it built, such as `Player_000100`.
    pub name: WString,
    _unk0138: [u8; 0x240],
    /// The character's `ChrAsmCtrl`, which owns its equipment.
    ///
    /// The vtable getter that returns it is `CharacterCtrl`'s own, inherited by `PlayerCtrl`, so the
    /// field belongs to this class even though only the local player's is read today. That getter
    /// is vtable slot `0x120`, `0x1403126b0`, in full `48 8b 81 78 03 00 00 c3` --
    /// `mov rax,[rcx+0x378]; ret`.
    pub chr_asm_ctrl: Option<NonNull<ChrAsmCtrl>>,
    _unk0380: [u8; 0x100],
}

/// `PlayerCtrl`, the controller the local player, a remote player and a bloodstain replay share.
///
/// Source of name: RTTI and DLRF.
#[repr(C)]
pub struct PlayerCtrl {
    /// The `CharacterCtrl` part.
    pub base: CharacterCtrl,
    _unk0480: [u8; 0x10],
    /// The character's `PlayerParam`: level, stats, souls and soul memory.
    ///
    /// The game's own getter `FUN_1401ab660` is the whole path from the manager, in full
    /// `mov rax,[0x1416148f0]; test; jz; mov rax,[rax+0xd0]; test; jz; mov rax,[rax+0x490]; ret`.
    pub player_param: Option<NonNull<PlayerParam>>,
    _unk0498: [u8; 0x08],
}

/// `PlayerParam`, a character's levelled state. A prefix: only the fields this repo reads.
///
/// Source of name: the Cheat Engine tables and `scripts/ds2-sl2.py`; there is no RTTI for it.
#[repr(C)]
pub struct PlayerParam {
    _unk00: [u8; 0x08],
    /// The nine levelled stats in the game's order, which is not the planner's: vigor, endurance,
    /// vitality, attunement, strength, dexterity, intelligence, faith, adaptability. See
    /// `ds2_rva::PLAYER_PARAM_STAT_OFFSETS`.
    pub stats: [u16; 9],
    _unk001a: [u8; 0xb6],
    /// Soul level.
    pub soul_level: u32,
    _unk00d4: [u8; 0x18],
    /// Souls held.
    pub souls_held: u32,
    _unk00f0: [u8; 0x04],
    /// Soul memory, first copy. The game keeps two; a write has to update both.
    pub soul_memory: u32,
    _unk00f8: [u8; 0x04],
    /// Soul memory, second copy.
    pub soul_memory_2: u32,
}

#[cfg(windows)]
impl PlayerCtrl {
    /// The character's `PlayerParam`, or `None` when the pointer is null or unreadable.
    pub fn player_param(player: NonNull<Self>) -> Option<NonNull<PlayerParam>> {
        let field = player.as_ptr() as usize + core::mem::offset_of!(Self, player_param);
        // SAFETY: a fault-safe read of one pointer-sized field; `None` on an unmapped address.
        let param = unsafe { ds2_game_base::mem::safe_read_usize(field) }?;
        NonNull::new(param as *mut PlayerParam)
    }
}

/// The phantom block a [`CharacterCtrl`] owns. A prefix: only the field this repo reads.
#[repr(C)]
pub struct PhantomBlock {
    _unk00: [u8; 0x3c],
    /// The phantom param id. The engine's own replay test calls two of its values a replay; they
    /// are `ds2_rva::REPLAY_PHANTOM_PARAM_IDS`.
    pub phantom_param_id: u8,
}

/// `ChrAsmCtrl`, which owns a character's equipment. A prefix: only the field this repo reads.
///
/// Source of name: RTTI.
#[repr(C)]
pub struct ChrAsmCtrl {
    _unk00: [u8; 0x28],
    /// The equipment state the requirement check reads.
    ///
    /// `ChrAsmCtrl` vtable (`0x1410e0a38`) slot `0x70` is `0x1401513d0`, in full
    /// `48 8b 41 28 c3` -- `mov rax,[rcx+0x28]; ret`. The object is built by `FUN_140347970` into
    /// `this[5]` of `FUN_140338e40`, and it is what `FUN_14034a980` walks.
    pub equip: Option<NonNull<ChrAsmEquip>>,
}

/// The equipment state a [`ChrAsmCtrl`] holds. A prefix: only the field this repo reads.
///
/// Source of name: none. The object has no vtable and no RTTI; it is named for what its owner
/// uses it for.
#[repr(C)]
pub struct ChrAsmEquip {
    _unk00: [u8; 0x10],
    /// The grip state: `1` one-handed, `2` and `3` two-handed, `4` to `6` power stance.
    ///
    /// `FUN_140347970` stores `1` there at `0x140347994`; `FUN_14034f470` rewrites it from the
    /// power-stance resolver `FUN_140350170`; `FUN_14034a980` passes it to the mechanics
    /// requirement check `FUN_14034d3c0`, which halves the Strength requirement (`u16`, `shr cx,1`
    /// at `0x14034d44c`) when `grip - 2 < 2`. The two-handed values are
    /// `ds2_rva::EQUIP_GRIP_TWO_HANDED`.
    pub grip: i32,
}

/// The MSVC `std::wstring` the game's CRT uses, with small-string optimisation.
///
/// The first field is a union: the characters inline while [`WString::capacity`] is at most
/// `ds2_rva::WSTRING_SSO_MAX`, a pointer to them above it.
#[repr(C)]
pub struct WString {
    storage: [u8; 0x10],
    /// Length in `wchar_t`, excluding the terminator.
    pub len: usize,
    /// Capacity in `wchar_t`; decides which side of the storage union is live.
    pub capacity: usize,
}

#[cfg(test)]
mod tests {
    use core::mem::{offset_of, size_of};

    use super::{
        CharacterCtrl, CharacterCtrlBase, ChrAsmCtrl, ChrAsmEquip, PhantomBlock, PlayerCtrl,
        PlayerParam, WString,
    };

    /// The `PlayerParam` hop and fields land where `ds2-rva` and the game's getter put them.
    #[test]
    fn player_param_fields_match_ds2_rva() {
        assert_eq!(
            offset_of!(PlayerCtrl, player_param),
            ds2_rva::PLAYER_PARAM_OFFSET
        );
        assert_eq!(
            offset_of!(PlayerParam, stats),
            ds2_rva::PLAYER_PARAM_STAT_OFFSETS[0]
        );
        assert_eq!(
            offset_of!(PlayerParam, stats) + size_of::<[u16; 8]>(),
            ds2_rva::PLAYER_PARAM_STAT_OFFSETS[8]
        );
        assert_eq!(
            offset_of!(PlayerParam, soul_level),
            ds2_rva::PLAYER_PARAM_SOUL_LEVEL_OFFSET
        );
        assert_eq!(
            offset_of!(PlayerParam, souls_held),
            ds2_rva::PLAYER_PARAM_SOULS_HELD_OFFSET
        );
        assert_eq!(
            offset_of!(PlayerParam, soul_memory),
            ds2_rva::PLAYER_PARAM_SOUL_MEMORY_OFFSETS[0]
        );
        assert_eq!(
            offset_of!(PlayerParam, soul_memory_2),
            ds2_rva::PLAYER_PARAM_SOUL_MEMORY_OFFSETS[1]
        );
    }

    // Sizes: each is the value slot 9 of the class's DLRF runtime-class vtable returns, and the
    // game's own allocations agree. `FUN_140357920` allocates 0x4a0 and passes it to the PlayerCtrl
    // constructor `FUN_14037ebe0`, which zeroes 0x480..0x4a0 after running the CharacterCtrl
    // constructor. `FUN_140355930` and `FUN_1403560a0` allocate 0x480 for `FUN_1403114f0`, the
    // CharacterCtrl constructor. CharacterCtrlBase has no allocation of its own that was found:
    // its constructor `FUN_140315a10` writes up to +0x55, and `FUN_1403114f0` writes its first
    // own field at +0x58 straight after calling it.

    #[test]
    fn character_ctrl_base_is_0x58() {
        assert_eq!(size_of::<CharacterCtrlBase>(), 0x58);
    }

    #[test]
    fn character_ctrl_is_0x480() {
        assert_eq!(size_of::<CharacterCtrl>(), 0x480);
    }

    #[test]
    fn player_ctrl_is_0x4a0() {
        assert_eq!(size_of::<PlayerCtrl>(), 0x4a0);
    }

    // Offsets: each is the value `ds2-rva` holds for the field, read live on 2026-09-26 from the
    // local player with `scripts/frida/player-ctrl.js` (a PlayerCtrl vtable, a finite position
    // with w = 1, a non-null phantom block and ChrAsmCtrl, and the name `Player_000100`).

    #[test]
    fn vtable_is_first() {
        assert_eq!(offset_of!(CharacterCtrlBase, vtable), 0x00);
        assert_eq!(offset_of!(CharacterCtrl, base), 0x00);
        assert_eq!(offset_of!(PlayerCtrl, base), 0x00);
    }

    #[test]
    fn character_ctrl_position_is_at_0x90() {
        assert_eq!(offset_of!(CharacterCtrl, position), 0x90);
    }

    #[test]
    fn character_ctrl_phantom_block_is_at_0xb0() {
        assert_eq!(offset_of!(CharacterCtrl, phantom_block), 0xb0);
    }

    #[test]
    fn character_ctrl_name_is_at_0x118() {
        assert_eq!(offset_of!(CharacterCtrl, name), 0x118);
    }

    #[test]
    fn character_ctrl_chr_asm_ctrl_is_at_0x378() {
        assert_eq!(offset_of!(CharacterCtrl, chr_asm_ctrl), 0x378);
    }

    // Read live on 2026-09-26 by `scripts/frida/player-ctrl.js`: the local player's ChrAsmCtrl had
    // vtable `0x10e0a38`, a non-null equip pointer at +0x28, and a grip of `3` there at +0x10.

    #[test]
    fn chr_asm_ctrl_equip_is_at_0x28() {
        assert_eq!(offset_of!(ChrAsmCtrl, equip), 0x28);
    }

    #[test]
    fn chr_asm_equip_grip_is_at_0x10() {
        assert_eq!(offset_of!(ChrAsmEquip, grip), 0x10);
    }

    #[test]
    fn phantom_param_id_is_at_0x3c() {
        assert_eq!(offset_of!(PhantomBlock, phantom_param_id), 0x3c);
    }

    #[test]
    fn wstring_len_and_capacity_follow_the_storage() {
        assert_eq!(offset_of!(WString, len), 0x10);
        assert_eq!(offset_of!(WString, capacity), 0x18);
        assert_eq!(size_of::<WString>(), 0x20);
    }
}
