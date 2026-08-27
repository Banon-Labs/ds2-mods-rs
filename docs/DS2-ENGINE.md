# What the DS2 binary actually is

Measured from `darksoulsii-deobf.bin` (build 9527516). Everything here is a count or a string
read out of the image — nothing is inferred from what Elden Ring does.

## It is heavily symbolized

**5271 MSVC RTTI type descriptors.** Class identification in Ghidra is a matter of reading
names, not of inferring them from vtable shape. This is the single biggest thing working in
this port's favour and it was not anticipated.

## DLRF is present — 587 registered runtime classes

This corrects an earlier claim in `PORTING.md` that DS2 "predates the DLRF reflection framework"
and that "the reflection metadata doesn't exist in DS2". It does exist:

```
DLRuntimeClassImpl registrations   587
DLUT@@ occurrences                 934
DLRF@@                             826
DLKR@@                             122
DLIO@@                              56
DLTX@@                              10
FD4                                  1
```

So the correct statement is narrower: **FD4 is absent** (one incidental occurrence), but the
`DL*` framework family — `DLRF` reflection, `DLUT`, `DLKR`, `DLIO`, `DLTX` — is all here. DS2
predates FD4 specifically, not the reflection stack. A `darksouls2` bindings crate has metadata
to be generated from, which is materially more tractable than "every structure by hand".

## The menu system is `Fe*`, and there is no Scaleform

`grep -ic scaleform` over the image returns **0**. `er-scaleform-hooks` and `er-gfx` are
confirmed inapplicable — not "unverified" as `PORTING.md` first recorded.

The player-facing UI is a front-end framework whose classes are DLRF-registered:

```
.?AV?$DLRuntimeClassImpl@VFeObjectBase@@$0A@@DLRF@@
.?AV?$DLRuntimeClassImpl@VFeObjectButton@@$0A@@DLRF@@
.?AV?$DLRuntimeClassImpl@VFeObjectButtonEx@@$0A@@DLRF@@
.?AV?$DLRuntimeClassImpl@VFeObjectWriteWordSelectButton@@$0A@@DLRF@@
.?AV?$DLRuntimeClassImpl@VFeBindResourceFileResourceObject@@$0A@@DLRF@@
```

plus `FeDynamicSystem`, `FeLaunchSystem`, `FeDynamicResource`, `FeDynamicCreateJob`, and menu
groups such as `FeGroupInGameMenuInventory2` and `FeGroupNpcMenuNumSelect`.

Button callbacks appear to bind through job functors. One RTTI descriptor spells the shape out:

```
FeFunctorJobCreator<JOB_CREATOR_MEMFUN_ARG2<
    FeGroupInGameMenuInventory2,
    DLUT::DLReferencePointer<FeJob>,
    FeDynamicMessageData,
    FeGroupNpcMenuNumSelect*>>
```

**Do not confuse the `GUI*` classes with this.** `GUIButton`, `GUIComboBox`, `GUIColorTweaker`,
`GUINumericEditBox`, `GUIOnOffTweaker`, `AppFlverIblTextureGUIProxy` are a separate developer
tooling UI left in the shipped build. Tweakers and colour selectors are not player menus.

## Still unverified

- Whether a MinHook detour survives the 48 Arxan stubs' integrity checks. Nothing has been
  hooked yet. This is the gate for every feature, not just menu work.
- Where menu text comes from. `FeDynamicMessageData` exists, but no text id space has been
  traced, and DS2's menu strings live in the game's archives rather than the exe.
