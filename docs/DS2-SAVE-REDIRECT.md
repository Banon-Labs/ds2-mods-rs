# Moving the save directory

How DARK SOULS II builds the path to `DS2SOFS0000.sl2`, which function to detour to move it, why
that one rather than the obvious one, and the second thing that turns out to be required.

Everything down to "Using it" was read out of `darksoulsii-deobf.bin` and the Ghidra project and
needed no run. The Steam ID section below is the opposite: three measured runs, and the reason it
is written as a table of runs rather than as a conclusion is that static analysis did not predict
it. The save carries its owner's account ID and the game checks it -- which no amount of reading
the path builder would have shown.

## The path, as the game builds it

Three string constants, found by searching the image for UTF-16 runs:

| string | VA | note |
| --- | --- | --- |
| `\DarkSoulsII\` | `0x1410d04a8` | one code reference |
| `DS2SOFS` | `0x1410da4d8` | the save file's base name |
| `.sl2` | `0x1411b5638` | |
| `\` | `0x1410d04f8` | the separator `FUN_140248db0` appends |

**There is no `DARKSII` string anywhere in the image, in either encoding.** That is the mechanical
reason a vanilla Dark Souls II save cannot be loaded by SOTFS however it is placed: the game never
constructs that file name. (It is also encrypted with a different key -- see `scripts/ds2-sl2.py
--vanilla`.)

Two functions build every DS2 user path:

```text
FUN_140248e80(std::wstring *out)                      // SAVE_APPDATA_ROOT_BUILD, RVA 0x00248e80
    SHGetFolderPathW(0, 0x1a /* CSIDL_APPDATA */, 0, 0)
    out  = that
    out += L"\\DarkSoulsII\\"

FUN_140248db0(std::wstring *out, const wchar_t *subdir)   // SAVE_DIR_BUILD, RVA 0x00248db0
    FUN_140248e80(out)
    out += subdir
    out += L"\\"
```

and one caller of each:

* `FUN_140248d80(out)` -- calls the root builder, appends `GraphicsConfig_SOFS.xml`.
* `FUN_140248db0` -- called only from `FUN_1402e6230_saveLoadSetup__` (`0x1402e635c`) and
  `FUN_1402e67f0` (`0x1402e6930`), both `SaveLoadSystem` methods.

That is exactly the layout on disk, in a live prefix and in a downloaded save alike:

```text
DarkSoulsII/GraphicsConfig_SOFS.xml
DarkSoulsII/<steamid hex>/DS2SOFS0000.sl2
```

## `subdir` is the Steam ID, and the call site says so

```text
0x1402e6331:  call   QWORD PTR [r8+0x38]           // vtable slot +0x38
0x1402e633c:  call   0x140a3de90                   // ... to a string
0x1402e634a:  cmp    QWORD PTR [rbp-0x19],0x8      // the wstring's capacity field
0x1402e634f:  lea    rdx,[rbp-0x31]                // so rdx is the inline buffer,
0x1402e6353:  cmovae rdx,QWORD PTR [rbp-0x31]      // ... or the heap pointer when it spilled
0x1402e6358:  lea    rcx,[rbp-0x1]                 // the out string
0x1402e635c:  call   0x140248db0
```

Slot `+0x38` is the same one `FUN_140af14e0` calls to fill the cached Steam ID at `DAT_1416681a8`.
The folder name is the account's SteamID64 **in hex**, not decimal -- `0x01100001018fa4be` for
`76561197986456766`. (Elden Ring uses decimal in the same position; do not carry that assumption
across.)

## Why the hook is on `db0`, not `e80`

`e80` is the wider chokepoint and the tempting one: it is the only thing in the image that turns
`SHGetFolderPathW` into a DARK SOULS II path. It is also **shared with the graphics config**, so a
detour there moves `GraphicsConfig_SOFS.xml` as well as the saves.

Worse, it leaves the Steam ID append downstream and untouched. Redirecting the root means the game
still builds `<newroot>\<your steam id>\`, so a donor save would have to be renamed out of its own
folder into yours. Hooking `db0` replaces root and ID folder together, so a donor save keeps
`01100001526d6d84` and nothing is renamed.

Both sites are clean: `scripts/ds2-arxan-chain.py` terminates at hop 0 for each, `48 89 5c 24 08`
at `db0` and `48 89 5c 24 10` at `e80`. Neither is one of the 286 Arxan-redirected entries.

`db0` returns nothing. Its epilogue is `mov rbx,[rsp+0x60]; add rsp,0x50; pop rdi; ret`, with no
load of `rax` on either path -- including the `subdir == NULL` early-out at `0x140248e6c` -- so
there is no return value for a caller to depend on and the detour is `void`.

## The out-parameter is a real `std::wstring`

Stock MSVC small-string optimisation, read out of the game's own string helpers
(`FUN_1400260b0` assign, `FUN_140043050` append):

```text
+0x00  16 bytes: the characters inline, OR a pointer to them
+0x10  length, in wchar_t, excluding the terminator
+0x18  capacity; at or below 7 the characters are inline, above it +0x00 is a pointer
```

The detour calls the game's own `assign` (`WSTRING_ASSIGN`, RVA `0x000260b0`) rather than writing
those three fields. The string is owned by the caller and may already hold an allocation from the
game's allocator; writing the fields by hand would either leak it or invite a free from the wrong
heap. `assign` is the same function the original uses to seat its `SHGetFolderPathW` result.

## Using it

There is no config key and no launcher flag. There was a `[save_redirect] path = ...` key and a
`--save-redirect` flag, and both are deleted, because what they did was not what they said.

The help string said *load the save at WINPATH*. What happened was: the DLL copied that file into
`ds2-save-staging/` beside the executable, pointed the game at that **directory**, and rewrote the
copy from the source on the next launch. The file named was never opened by the game, and a session
started that way threw away everything done in it -- the copy is what the game saved into, and the
copy is what the next launch overwrote. It also armed both sides at once for the whole session, so
anything else being tested in that session was silently reading and writing the staged duplicate.

What remains is the pause menu's **Load Character from File** row, which is the feature this was
standing in for, and the handoff file that row writes when it cannot do the swap in-session. Four
shapes are accepted, told apart by extension: a bare `.sl2`, or a `.zip`/`.7z`/`.rar` containing
**exactly one** `DS2SOFS0000.sl2` at any depth. Zero copies or several are refused by name rather
than resolved by picking the first -- a downloaded archive may hold the save at
`DarkSoulsII/<steamid>/DS2SOFS0000.sl2` or bare at the root, and both occur in the wild.

### The DLL does the rebind, and needs nothing from you

The source file is never modified. On the detour's first call the DLL extracts the save, rewrites
the Steam ID inside it, and writes the result to `ds2-save-staging/` beside the executable -- then
hands the game that directory.

The ID it writes is the one **the game handed the hooked function as its second argument**. That is
the whole reason this belongs in the DLL rather than a script: nobody has to look their SteamID64
up, convert it to hex, or get it wrong.

Staging happens on the first detour call rather than at install time, deliberately: that is on the
game thread with the save system already up, which is a far better place for archive decompression
and file writes than a loader callback running before the entry point.

### The staged copy is rewritten every launch

It is what the game reads **and writes**, so progress made in a redirected run lives in
`ds2-save-staging` and does not survive the next launch. That is what pointing at a read-only
source means -- "start from this save", not "adopt this save".

### It fails open

A source that cannot be read, an archive with no save in it, a save whose ID cannot be rewritten:
all of these fall back to the game's own save directory and say so loudly in the log. A bootable
game with a shouted warning beats a game that will not start.

```text
ds2-save-redirect: staged kind=zip bytes=8251680 replaced=1 previous=01100001526d6d84 dir=...
ds2-save-redirect: save-dir redirected steam-id=01100001018fa4be path=...\ds2-save-staging\
ds2-save-redirect: stage-failed source=... error=... -- FALLING BACK to the game's own save directory
```

The redirect line reads the string back out of the game after the call rather than echoing intent.
DS2 draws no LOAD GAME row when it finds no save, so "pointed at the wrong folder" and "there was
never a save there" are indistinguishable on screen and differ only here.

### The slot belongs to the save, not to your account

`--continue-slot` is resolved against the redirected save before launching: with no value given,
the first occupied slot is used; with a value the file says is empty, the launch is warned about
but not blocked, because the runtime applies an ownership check the launcher cannot see.

Slots are reported `occupied`, `placeholder` or `empty`. **`placeholder`** means the nine stats are
all `1` -- initialised but holding no character. That state is real and common: it is a save's next
free slot, and a downloaded "mule" had it in all ten. Reading it as occupied made that mule look
like ten characters, which is why the classification exists rather than a bare non-zero test.

Occupancy is judged from stat content because it must be: the record's own flags byte at `+0x1D9`
is zero in every real `.sl2`, since the game derives it when it loads the per-character entries.

## The redirect alone is not enough: the save carries a Steam ID

**Measured across three runs, not reasoned about.**

| run | `path` | result |
| --- | --- | --- |
| donor save, donor folder | `Z:\home\banon\DS2\DarkSoulsII\01100001526d6d84` | *"The save data was not loaded correctly. Create new save data?"* |
| this account's own save, via the same `Z:` mechanism | `Z:\home\banon\.local\share\Steam\steamapps\compatdata\335300\pfx\drive_c\users\steamuser\AppData\Roaming\DarkSoulsII\01100001018fa4be` | boots straight through, no dialog |
| donor save with its Steam ID rewritten | `Z:\home\banon\DS2\DarkSoulsII\01100001018fa4be` | **loads into the world** |

The middle run is the control that makes the first one mean something: it isolates a single
variable. `Z:` is reachable from inside the prefix, the hook substitutes the path correctly, and a
134-character path is fine. What the game rejects is the file's **contents**.

DS2 SOTFS writes the owning account's SteamID64 **as ASCII hex** into the decrypted payload of
`USER_DATA000` at offset `0x39` -- exactly one occurrence in the whole save, in every file examined.
The game compares it and refuses a mismatch. Note the folder name is irrelevant once this hook is
installed: the game is handed a complete path and never parses the directory it was given.

`scripts/ds2-sl2-rebind.py` rewrites it:

```bash
python3 scripts/ds2-sl2-rebind.py <donor>.sl2 --show
python3 scripts/ds2-sl2-rebind.py <donor>.sl2 --to 01100001018fa4be -o <out>.sl2
```

The patch is length-preserving -- a SteamID64 in hex is always 16 characters -- so no section size
changes and no offset table is rewritten. The reseal is the part that is easy to get wrong: the
entry is `[16-byte MD5][16-byte IV][AES-128-CBC ciphertext]`, the MD5 covers the **ciphertext with
its IV** rather than the plaintext, and the entry must be re-encrypted under its **own original
IV** so that everything outside the patched block stays byte-identical. Getting the hash wrong
produces a file the game rejects in exactly the same way as the unpatched one. The tool re-reads
its own output before writing -- every MD5, every section chain, and the ID set -- and refuses to
write a save that fails.

Find your own ID from the folder the game already uses: `...\AppData\Roaming\DarkSoulsII\<id>\`.
It is the SteamID64 in **hex**, not decimal.

## Ordering, and the thing that is not optional

`install_save_redirect` runs **after** `install_offline` in the post-Arxan callback. Loading someone
else's save is the shape of thing FromSoftware's matchmaking watches for, so the flag patches and
the import-table guard should already be in place before the save system exists. `[offline]`
defaults to on for that reason and should stay on for any run that uses this.

## Mid-session, the lever is the storage worker

Everything above is a launch-time redirect and cannot move a live session: `SAVE_DIR_BUILD` runs
during session setup, so re-pointing it once the game is up rewrites a string nothing reads again.
That is not a dead end, though, because of *where* its result goes. The `0x18` arm of the session
pump (`FUN_1402e6230`, at `0x1402e635c`) is the whole of it:

```text
FUN_140248db0(&dir, steamid);                     // SAVE_DIR_BUILD
if (!SLSystem->field_0x1a1) {                     // a once-per-process latch
    FUN_140a899f0(SLSystem->_x38, 0, dir);        //   first session:       index 0
    SLSystem->field_0x1a1 = true;
} else {
    FUN_140a899f0(SLSystem->_x38, 1, dir);        //   every later session: index 1
}
```

`FUN_140a899f0` seats the directory on the storage worker, at `worker+0x48`, and that is the field
a container read opens. Since the launch-time redirect was measured reading a donor container end
to end, and this is the only route its string can have taken, the worker's copy is the one that
matters -- so an in-session redirect calls `FUN_140a899f0` directly rather than trying to make
session setup happen again. `ds2_save_redirect::request_dir` is that call.

### Three seams that are not it

| seam | what it moves | how it was disproved |
|---|---|---|
| `SLLoadSession`'s directory virtual (`session_dir`) | the load class's vtable slot 3 | the work method reaches the same string through the accessor and never calls the virtual: `load-answered=1` on a read that still failed |
| `SAVE_DIR_BUILD` re-pointed mid-session | the string session setup builds | session setup does not re-run for a re-read: `session-dir-answered=0` |
| `SLLoadContent`'s own string, `[[system+0x30]]+0x08` | a field beside the worker's | measured `<unreadable>`, and it is not the string the worker holds |

### The set is called whole, and the read-back is a hook

`FUN_140a899f0` is a lock/unlock pair around one mutation: `FUN_140a8bfb0` takes the manager's lock
at `manager+0x50` and the worker's at `worker+0xb0` and leaves **both held**, and `FUN_140a8c390`
releases them. The worker pointer exists only between those two calls. Reaching for the finder
alone to read the directory back would leave the save system locked against its own next request,
so the read-back is a detour on the worker-side writer (`FUN_140a8d9b0`) instead -- the only writer
of that field, which means it also reports every directory the game sets on itself.

It reports one more thing that nothing else can see. `worker+0xad` is a byte the writer consults
first, and a non-zero value jumps the entire body: the index write, the status reset and the string
set, all of it. **A skipped set is otherwise completely silent**, which is the shape of failure the
three seams above all had.

## What is NOT done

No validation of the save being loaded. A guard that refuses a save whose soul memory is too low
for its level was scoped and partly reversed -- soul memory is `sPlayerParam+0xF4`
(`total_get_soul_1`, written by `addSoul` at `0x14038ab40`, capped at 999,999,999), and the
souls-per-level table is `PlayerLevelUpSoulsParam`, param index `0x27`, whose container is
`CharacterManager+0x580`. Two pieces are missing: the row format of `ParamFileResourceObject`, and
the attributes-to-level relation (DS2 has no level field -- `sPlayerParam` carries
`attributes: short[11]` and derives it; `RelatePhysicalStatToLevelStatParam`, index `0x28`, is the
likely home of the rule). No primary source was found for FromSoftware's actual ban criteria, so
the specification for such a guard is presently folklore rather than fact.
